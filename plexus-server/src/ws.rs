//! Client WebSocket handler: device login, heartbeat, tool registration, tool results.

use crate::consts::HEARTBEAT_REAPER_INTERVAL_SEC;
use crate::state::{AppState, DeviceConnection};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use plexus_common::consts::PROTOCOL_VERSION;
use plexus_common::protocol::{
    ClientToServer, FsPolicy, McpServerEntry, ServerToClient, ToolExecutionResult,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::Mutex;
use tracing::{info, warn};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_connection(socket, state))
}

async fn handle_connection(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Send RequireLogin
    let msg = ServerToClient::RequireLogin {
        message: "PLEXUS Server v1.0".into(),
    };
    if send_msg(&mut sink, &msg).await.is_err() {
        return;
    }

    // Await SubmitToken
    let (user_id, device_name, device_token) = match stream.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientToServer>(&text) {
            Ok(ClientToServer::SubmitToken {
                token,
                protocol_version,
            }) => {
                if protocol_version != PROTOCOL_VERSION {
                    let _ = send_msg(
                        &mut sink,
                        &ServerToClient::LoginFailed {
                            reason: format!(
                                "Protocol mismatch: expected {PROTOCOL_VERSION}, got {protocol_version}"
                            ),
                        },
                    )
                    .await;
                    return;
                }
                match crate::db::devices::find_by_token(&state.db, &token).await {
                    Ok(Some(dt)) => (dt.user_id.clone(), dt.device_name.clone(), dt),
                    _ => {
                        let _ = send_msg(
                            &mut sink,
                            &ServerToClient::LoginFailed {
                                reason: "Invalid token".into(),
                            },
                        )
                        .await;
                        return;
                    }
                }
            }
            _ => return,
        },
        _ => return,
    };

    // Send LoginSuccess
    let fs_policy: FsPolicy =
        serde_json::from_value(device_token.fs_policy.clone()).unwrap_or_default();
    let mcp_servers: Vec<McpServerEntry> =
        serde_json::from_value(device_token.mcp_config.clone()).unwrap_or_default();
    let ssrf_whitelist: Vec<String> =
        serde_json::from_value(device_token.ssrf_whitelist.clone()).unwrap_or_default();

    if send_msg(
        &mut sink,
        &ServerToClient::LoginSuccess {
            user_id: user_id.clone(),
            device_name: device_name.clone(),
            fs_policy,
            mcp_servers,
            workspace_path: device_token.workspace_path.clone(),
            shell_timeout_max: device_token.shell_timeout_max as u64,
            ssrf_whitelist,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    info!("Device connected: {user_id}:{device_name}");

    // Register device in state
    let device_key = AppState::device_key(&user_id, &device_name);
    let sink = Arc::new(Mutex::new(sink));
    let last_seen = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp()));

    state.devices.insert(
        device_key.clone(),
        DeviceConnection {
            user_id: user_id.clone(),
            device_name: device_name.clone(),
            sink: Arc::clone(&sink),
            last_seen: Arc::clone(&last_seen),
            tools: Vec::new(),
            tool_schemas: Vec::new(),
            mcp_schemas: Vec::new(),
        },
    );
    state
        .devices_by_user
        .entry(user_id.clone())
        .or_default()
        .push(device_key.clone());
    state.pending.entry(device_key.clone()).or_default();

    // Message loop
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else {
            continue;
        };
        let Ok(client_msg) = serde_json::from_str::<ClientToServer>(&text) else {
            warn!("Bad message from {device_key}");
            continue;
        };
        last_seen.store(chrono::Utc::now().timestamp(), Ordering::SeqCst);

        match client_msg {
            ClientToServer::Heartbeat { .. } => {
                let _ = send_msg_arc(&sink, &ServerToClient::HeartbeatAck).await;
            }
            ClientToServer::RegisterTools {
                tool_names,
                tool_schemas,
                mcp_schemas,
            } => {
                // FR6 / spec §4.6: compare incoming MCP schemas against
                // already-registered install sites (server MCPs + other
                // devices for this user). Reject only the conflicting MCP
                // entries; the rest of the registration proceeds so the
                // agent still sees the device's non-MCP tools (shell,
                // file tools) and any clean MCP servers.
                let accepted_mcp_schemas = if mcp_schemas.is_empty() {
                    Vec::new()
                } else {
                    validate_and_filter_mcp_schemas(
                        &state,
                        &user_id,
                        &device_key,
                        &device_name,
                        mcp_schemas,
                        &sink,
                    )
                    .await
                };

                // Drop any tool_names referring to rejected MCP servers so
                // the aggregated tool list (built in `tools_registry`) does
                // not emit a schema-less MCP tool.
                let accepted_servers: std::collections::HashSet<String> = accepted_mcp_schemas
                    .iter()
                    .map(|s| s.server.clone())
                    .collect();
                let filtered_tool_names: Vec<String> = tool_names
                    .into_iter()
                    .filter(|n| {
                        if let Some(rest) = n.strip_prefix("mcp_") {
                            // Keep only if SOME accepted server name is a
                            // prefix of `rest` (matches MCP wrap naming).
                            accepted_servers
                                .iter()
                                .any(|s| rest.starts_with(&format!("{s}_")))
                        } else {
                            true
                        }
                    })
                    .collect();

                if let Some(mut conn) = state.devices.get_mut(&device_key) {
                    conn.tools = filtered_tool_names;
                    conn.tool_schemas = tool_schemas;
                    conn.mcp_schemas = accepted_mcp_schemas;
                }
                state.tool_schema_cache.remove(&user_id);
                info!("Tools registered for {device_key}");
            }
            ClientToServer::ToolExecutionResult(result) => {
                resolve_pending(&state, &device_key, result);
            }
            ClientToServer::FileResponse {
                request_id,
                content_base64,
                mime_type,
                error,
            } => {
                let result = ToolExecutionResult {
                    request_id: request_id.clone(),
                    exit_code: if error.is_some() { 1 } else { 0 },
                    output: if let Some(e) = error {
                        e
                    } else {
                        serde_json::json!({
                            "content_base64": content_base64,
                            "mime_type": mime_type,
                        })
                        .to_string()
                    },
                };
                resolve_pending(&state, &device_key, result);
            }
            ClientToServer::FileSendAck { request_id, error } => {
                let result = ToolExecutionResult {
                    request_id: request_id.clone(),
                    exit_code: if error.is_some() { 1 } else { 0 },
                    output: error.unwrap_or_else(|| "ok".into()),
                };
                resolve_pending(&state, &device_key, result);
            }
            ClientToServer::StreamChunk {
                request_id, data, ..
            } => {
                crate::device_stream::dispatch_frame(
                    &state,
                    &device_key,
                    &request_id,
                    crate::device_stream::StreamFrame::Chunk(data),
                );
            }
            ClientToServer::StreamEnd { request_id, .. } => {
                crate::device_stream::dispatch_frame(
                    &state,
                    &device_key,
                    &request_id,
                    crate::device_stream::StreamFrame::End,
                );
            }
            ClientToServer::StreamError { request_id, error } => {
                crate::device_stream::dispatch_frame(
                    &state,
                    &device_key,
                    &request_id,
                    crate::device_stream::StreamFrame::Error(error),
                );
            }
            _ => {}
        }
    }

    // Disconnect cleanup
    info!("Device disconnected: {device_key}");
    state.devices.remove(&device_key);
    if let Some(mut keys) = state.devices_by_user.get_mut(&user_id) {
        keys.retain(|k| k != &device_key);
        if keys.is_empty() {
            drop(keys);
            state.devices_by_user.remove(&user_id);
        }
    }
    state.pending.remove(&device_key);
    state.streams.remove(&device_key);
    state.tool_schema_cache.remove(&user_id);
}

/// Pure split of incoming MCP schemas into `(accepted, rejected_servers,
/// conflict_json)` given a pre-gathered baseline of existing installs.
/// Extracted so the decision logic is unit-testable without any
/// live WebSocket or `AppState`.
pub(crate) fn partition_incoming_mcp_schemas(
    existing: &[crate::mcp::wrap::McpInstall],
    incoming: Vec<plexus_common::protocol::McpServerSchemas>,
    incoming_site: &str,
) -> (
    Vec<plexus_common::protocol::McpServerSchemas>,
    Vec<String>,
    Vec<serde_json::Value>,
) {
    let mut accepted: Vec<plexus_common::protocol::McpServerSchemas> = Vec::new();
    let mut rejected_servers: Vec<String> = Vec::new();
    let mut rejected_conflicts: Vec<serde_json::Value> = Vec::new();

    for entry in incoming {
        let incoming_install = crate::mcp::wrap::McpInstall {
            install_site: incoming_site.to_string(),
            mcp_server_name: entry.server.clone(),
            tools: entry
                .tools
                .iter()
                .map(|t| (t.name.clone(), t.parameters.clone()))
                .collect(),
        };
        let diffs = crate::mcp::wrap::diff_mcp_schema_collisions(existing, &incoming_install);
        if diffs.is_empty() {
            accepted.push(entry);
        } else {
            rejected_servers.push(entry.server.clone());
            for d in diffs {
                rejected_conflicts.push(serde_json::json!({
                    "mcp_server": entry.server,
                    "tool": d.tool,
                    "existing_schema": d.existing_schema,
                    "new_schema": d.new_schema,
                    "where_installed": d.where_installed,
                }));
            }
        }
    }
    (accepted, rejected_servers, rejected_conflicts)
}

/// Validate incoming MCP schemas against existing installs (server MCPs +
/// other devices) and filter out anything that collides. Conflicts are
/// reported back to the client via a single `RegisterToolsError` frame.
///
/// Returns the subset of `mcp_schemas` that are safe to register for this
/// device.
async fn validate_and_filter_mcp_schemas(
    state: &AppState,
    user_id: &str,
    device_key: &str,
    _device_name: &str,
    incoming: Vec<plexus_common::protocol::McpServerSchemas>,
    sink: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
) -> Vec<plexus_common::protocol::McpServerSchemas> {
    // Gather existing installs: server MCPs + every OTHER device for the
    // same user. (Re-registering the same device replaces its prior
    // entry, so we exclude its own cached schemas from the baseline.)
    let mut existing: Vec<crate::mcp::wrap::McpInstall> = Vec::new();
    {
        let server_mcp = state.server_mcp.read().await;
        for (server_name, tools) in server_mcp.raw_tool_schemas_by_server() {
            existing.push(crate::mcp::wrap::McpInstall {
                install_site: plexus_common::consts::SERVER_DEVICE_NAME.to_string(),
                mcp_server_name: server_name,
                tools,
            });
        }
    }
    if let Some(keys) = state.devices_by_user.get(user_id) {
        for key in keys.value() {
            if key == device_key {
                continue;
            }
            if let Some(conn) = state.devices.get(key) {
                let other_name = conn.device_name.clone();
                existing.extend(crate::mcp::wrap::installs_from_reported_schemas(
                    &other_name,
                    &conn.mcp_schemas,
                ));
            }
        }
    }

    let (accepted, rejected_servers, rejected_conflicts) =
        partition_incoming_mcp_schemas(&existing, incoming, device_key);

    if !rejected_conflicts.is_empty() {
        warn!("MCP schema collision(s) for {device_key}: rejecting servers {rejected_servers:?}");
        let err = ServerToClient::RegisterToolsError {
            code: "mcp_schema_collision".into(),
            message: format!(
                "MCP server(s) {:?} conflict with existing installs — rename or upgrade",
                rejected_servers
            ),
            conflicts: rejected_conflicts,
        };
        let _ = send_msg_arc(sink, &err).await;
    }

    accepted
}

fn resolve_pending(state: &AppState, device_key: &str, result: ToolExecutionResult) {
    if let Some(device_pending) = state.pending.get(device_key)
        && let Some((_, sender)) = device_pending.remove(&result.request_id)
    {
        let _ = sender.send(result);
    }
}

async fn send_msg(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &ServerToClient,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).unwrap();
    sink.send(Message::Text(json.into())).await.map_err(|_| ())
}

async fn send_msg_arc(
    sink: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    msg: &ServerToClient,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).unwrap();
    sink.lock()
        .await
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

/// Spawn heartbeat reaper: checks every 30s for stale devices.
pub fn spawn_heartbeat_reaper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            HEARTBEAT_REAPER_INTERVAL_SEC,
        ));
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    info!("heartbeat reaper shutting down");
                    break;
                }
                _ = interval.tick() => {
                    let now = chrono::Utc::now().timestamp();
                    let timeout = plexus_common::consts::HEARTBEAT_INTERVAL_SEC as i64 * 4;
                    let mut stale = Vec::new();
                    for entry in state.devices.iter() {
                        let last = entry.value().last_seen.load(Ordering::SeqCst);
                        if now - last > timeout {
                            stale.push(entry.key().clone());
                        }
                    }
                    for key in stale {
                        warn!("Reaping stale device: {key}");
                        if let Some((_, conn)) = state.devices.remove(&key) {
                            if let Some(mut keys) = state.devices_by_user.get_mut(&conn.user_id) {
                                keys.retain(|k| k != &key);
                                if keys.is_empty() {
                                    drop(keys);
                                    state.devices_by_user.remove(&conn.user_id);
                                }
                            }
                            state.pending.remove(&key);
                            state.streams.remove(&key);
                            state.tool_schema_cache.remove(&conn.user_id);
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    //! FR6 / spec §4.6: collision logic for `RegisterTools`. These exercise
    //! the pure-logic half of `validate_and_filter_mcp_schemas`
    //! (the existing-baseline assembly is covered separately via the
    //! `server_mcp` + `devices_by_user` call sites, which need a live
    //! `AppState`).

    use super::partition_incoming_mcp_schemas;
    use crate::mcp::wrap::McpInstall;
    use plexus_common::protocol::{McpRawTool, McpServerSchemas};
    use serde_json::json;

    fn sample_install(
        site: &str,
        server: &str,
        tool: &str,
        schema: serde_json::Value,
    ) -> McpInstall {
        McpInstall {
            install_site: site.into(),
            mcp_server_name: server.into(),
            tools: vec![(tool.into(), schema)],
        }
    }

    #[test]
    fn accepts_all_when_no_collision() {
        // Existing: server has foo.search{query: string}
        let existing = vec![sample_install(
            "server",
            "foo",
            "search",
            json!({"properties": {"query": {"type": "string"}}}),
        )];
        // Device reports the identical schema.
        let incoming = vec![McpServerSchemas {
            server: "foo".into(),
            tools: vec![McpRawTool {
                name: "search".into(),
                parameters: json!({"properties": {"query": {"type": "string"}}}),
            }],
        }];
        let (accepted, rejected, conflicts) =
            partition_incoming_mcp_schemas(&existing, incoming, "device-B");
        assert_eq!(accepted.len(), 1);
        assert!(rejected.is_empty());
        assert!(conflicts.is_empty());
    }

    #[test]
    fn rejects_collision_keeps_noncolliding() {
        // Two existing installs: server has foo.search{query}, server has bar.ping{msg}.
        let existing = vec![
            sample_install(
                "server",
                "foo",
                "search",
                json!({"properties": {"query": {"type": "string"}}}),
            ),
            sample_install(
                "server",
                "bar",
                "ping",
                json!({"properties": {"msg": {"type": "string"}}}),
            ),
        ];
        // Device reports two MCPs: foo collides, bar matches.
        let incoming = vec![
            McpServerSchemas {
                server: "foo".into(),
                tools: vec![McpRawTool {
                    name: "search".into(),
                    parameters: json!({"properties": {"query": {"type": "string"}, "engine": {"type": "string"}}}),
                }],
            },
            McpServerSchemas {
                server: "bar".into(),
                tools: vec![McpRawTool {
                    name: "ping".into(),
                    parameters: json!({"properties": {"msg": {"type": "string"}}}),
                }],
            },
        ];
        let (accepted, rejected, conflicts) =
            partition_incoming_mcp_schemas(&existing, incoming, "device-B");
        // bar accepted.
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].server, "bar");
        // foo rejected.
        assert_eq!(rejected, vec!["foo".to_string()]);
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c["mcp_server"], "foo");
        assert_eq!(c["tool"], "search");
        assert_eq!(c["where_installed"], json!(["server"]));
    }

    #[test]
    fn second_device_with_same_schema_is_fine() {
        // Device A already has foo.search. Device B registers the same schema.
        let existing = vec![sample_install(
            "device-A",
            "foo",
            "search",
            json!({"properties": {"query": {"type": "string"}}}),
        )];
        let incoming = vec![McpServerSchemas {
            server: "foo".into(),
            tools: vec![McpRawTool {
                name: "search".into(),
                parameters: json!({"properties": {"query": {"type": "string"}}}),
            }],
        }];
        let (accepted, rejected, _) =
            partition_incoming_mcp_schemas(&existing, incoming, "device-B");
        assert_eq!(accepted.len(), 1);
        assert!(rejected.is_empty());
    }

    #[test]
    fn divergent_device_install_is_rejected() {
        // Device A has foo.search{query}. Device B registers divergent schema.
        let existing = vec![sample_install(
            "device-A",
            "foo",
            "search",
            json!({"properties": {"query": {"type": "string"}}}),
        )];
        let incoming = vec![McpServerSchemas {
            server: "foo".into(),
            tools: vec![McpRawTool {
                name: "search".into(),
                parameters: json!({"properties": {"query": {"type": "string"}, "engine": {"type": "string"}}}),
            }],
        }];
        let (accepted, rejected, conflicts) =
            partition_incoming_mcp_schemas(&existing, incoming, "device-B");
        assert!(accepted.is_empty());
        assert_eq!(rejected, vec!["foo".to_string()]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0]["where_installed"], json!(["device-A"]));
    }
}
