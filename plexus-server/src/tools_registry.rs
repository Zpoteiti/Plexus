//! Tool schema merging, device_name injection, and tool call routing.
//!
//! Three categories:
//! - Native server tools (e.g. web_fetch): no device_name, emitted as-is.
//! - MCP tools (mcp_{server}_{tool}): from server MCP and/or client devices.
//!   Dedup key = full prefixed name. device_name enum = "server" + any client
//!   devices that also have it. mcp_minimax_* and mcp_anthropic_* never merged.
//! - Client native tools (e.g. read_file): device_name enum = all client devices
//!   that have the tool.

use crate::consts::TOOL_EXECUTION_TIMEOUT_SEC;
use crate::state::AppState;
use futures_util::SinkExt;
use plexus_common::consts::SERVER_DEVICE_NAME;
use plexus_common::protocol::{ExecuteToolRequest, ServerToClient, ToolExecutionResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

/// Build merged tool schemas for a user. Uses cache if available.
pub fn build_tool_schemas(state: &AppState, user_id: &str) -> Vec<Value> {
    // Check cache first
    if let Some(cached) = state.tool_schema_cache.get(user_id) {
        return cached.as_ref().clone();
    }

    let mut schemas = Vec::new();

    // 1. Native server tools — no device_name, emitted as-is.
    schemas.extend(crate::server_tools::tool_schemas());

    // 2. Collect all client device tools: (device_name, tool_names, tool_schemas)
    let mut device_tools: Vec<(String, Vec<String>)> = Vec::new();
    // device_name -> [client-advertised schema] (e.g. `shell`).
    let mut device_client_schemas: Vec<(String, Vec<Value>)> = Vec::new();
    if let Some(keys) = state.devices_by_user.get(user_id) {
        for key in keys.value() {
            if let Some(conn) = state.devices.get(key) {
                device_tools.push((conn.device_name.clone(), conn.tools.clone()));
                device_client_schemas.push((conn.device_name.clone(), conn.tool_schemas.clone()));
            }
        }
    }

    // 3. Build accumulation maps keyed by tool name:
    //    - mcp_sources: mcp_* tools → ordered list of device names (may include "server")
    //    - native_sources: non-mcp client tools → ordered list of device names (names only)
    //    - representatives: first MCP schema seen for each key (used as emit template)
    let mut mcp_sources: HashMap<String, Vec<String>> = HashMap::new();
    let mut native_sources: HashMap<String, Vec<String>> = HashMap::new();
    let mut representatives: HashMap<String, Value> = HashMap::new();

    // 3a. Seed mcp_sources from server MCP (SERVER_DEVICE_NAME device).
    let server_mcp_schemas = match state.server_mcp.try_read() {
        Ok(g) => g.tool_schemas(),
        Err(_) => {
            warn!("build_tool_schemas: server_mcp lock contention — MCP tools excluded this call");
            vec![]
        }
    };
    for schema in server_mcp_schemas {
        let name = tool_name(&schema).to_string();
        if name.is_empty() {
            continue;
        }
        mcp_sources
            .entry(name.clone())
            .or_default()
            .push(SERVER_DEVICE_NAME.to_string());
        representatives.entry(name).or_insert(schema);
    }

    // 3b. Accumulate client tools into mcp_sources or native_sources.
    for (device_name, tools) in &device_tools {
        for name in tools {
            if name.is_empty() {
                continue;
            }
            if name.starts_with("mcp_") {
                mcp_sources
                    .entry(name.clone())
                    .or_default()
                    .push(device_name.clone());
            } else {
                native_sources
                    .entry(name.clone())
                    .or_default()
                    .push(device_name.clone());
            }
        }
    }

    // 1.5. File tools — unified schemas with device_name enum covering "server" + every
    //      client device that reports the capability. Always emitted (even when no client
    //      is online) with enum = ["server"] as the minimum.
    {
        use plexus_common::file_ops_schemas;

        for schema_fn in &[
            file_ops_schemas::read_file_schema as fn() -> Value,
            file_ops_schemas::write_file_schema,
            file_ops_schemas::edit_file_schema,
            file_ops_schemas::delete_file_schema,
            file_ops_schemas::list_dir_schema,
            file_ops_schemas::glob_schema,
            file_ops_schemas::grep_schema,
        ] {
            let mut schema = schema_fn();
            let tool_name_str = tool_name(&schema).to_string();
            // device_name enum = "server" + any client device that reports this tool.
            let mut devices: Vec<String> = vec!["server".to_string()];
            for (device_name, tools) in &device_tools {
                let has_tool = tools.iter().any(|t| t == &tool_name_str);
                if has_tool {
                    devices.push(device_name.clone());
                }
            }
            inject_device_name_enum(&mut schema, &devices);
            schemas.push(schema);
        }
    }

    // 4. Emit MCP tools — one schema per unique mcp_* name, device_name enum = all sources.
    let mut mcp_keys: Vec<String> = mcp_sources.keys().cloned().collect();
    mcp_keys.sort();
    for name in mcp_keys {
        let devices = &mcp_sources[&name];
        if let Some(template) = representatives.get(&name) {
            let mut schema = template.clone();
            inject_device_name_enum(&mut schema, devices);
            schemas.push(schema);
        }
    }

    // 5. Client-only tools (e.g. shell) — schemas are advertised per-device by
    //    `ClientToServer::RegisterTools::tool_schemas` and cached in
    //    `DeviceConnection::tool_schemas`. Group schemas by tool name across
    //    every connected client, inject a `device_name` enum of the reporting
    //    devices, and emit one aggregate tool per name.
    //
    //    Collision rule: if two devices report the same tool name with
    //    different schemas, keep the first-seen schema, log a warning, and
    //    still include the diverging device's name in the enum (visible
    //    failure — the agent will see the tool but may hit an arg mismatch
    //    at call time, which beats silently hiding the tool).
    let mut client_tool_groups: HashMap<String, (Value, Vec<String>)> = HashMap::new();
    for (device_name, device_schemas) in &device_client_schemas {
        for schema in device_schemas {
            let name = tool_name(schema).to_string();
            if name.is_empty() {
                continue;
            }
            match client_tool_groups.get_mut(&name) {
                Some((existing, devices)) => {
                    if existing != schema {
                        warn!(
                            "Client-only tool '{name}' reported with divergent schema by device '{device_name}' — keeping first-seen schema; agent may hit arg mismatch at call time"
                        );
                    }
                    devices.push(device_name.clone());
                }
                None => {
                    client_tool_groups.insert(name, (schema.clone(), vec![device_name.clone()]));
                }
            }
        }
    }

    let mut client_tool_keys: Vec<String> = client_tool_groups.keys().cloned().collect();
    client_tool_keys.sort();
    for name in client_tool_keys {
        let (template, devices) = &client_tool_groups[&name];
        let mut schema = template.clone();
        inject_device_name_enum(&mut schema, devices);
        schemas.push(schema);
    }

    // `native_sources` (tool NAMES reported by clients for non-mcp tools) is
    // intentionally ignored here for unknown tools — the canonical schema lives
    // with the reporting client now. Any client-only tool that isn't advertised
    // in `tool_schemas` is silently dropped from the aggregated list.
    let _ = native_sources;

    // Cache result
    state
        .tool_schema_cache
        .insert(user_id.to_string(), Arc::new(schemas.clone()));

    schemas
}

fn tool_name(schema: &Value) -> &str {
    schema
        .get("function")
        .and_then(|f| f.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("")
}

/// Inject device_name enum into a tool schema's parameters.
fn inject_device_name_enum(schema: &mut Value, devices: &[String]) {
    if let Some(function) = schema.get_mut("function")
        && let Some(parameters) = function.get_mut("parameters")
    {
        if let Some(properties) = parameters.get_mut("properties")
            && let Some(obj) = properties.as_object_mut()
        {
            obj.insert(
                "device_name".to_string(),
                serde_json::json!({
                    "type": "string",
                    "enum": devices,
                    "description": "Target device to execute this tool on"
                }),
            );
        }
        // Add device_name to required
        if let Some(required) = parameters.get_mut("required")
            && let Some(arr) = required.as_array_mut()
        {
            arr.push(Value::String("device_name".into()));
        }
    }
}

/// Route a tool call to the correct device. Returns the tool execution result.
pub async fn route_to_device(
    state: &Arc<AppState>,
    user_id: &str,
    device_name: &str,
    tool_name: &str,
    arguments: Value,
) -> ToolExecutionResult {
    let device_key = AppState::device_key(user_id, device_name);

    // Get device connection
    let conn = match state.devices.get(&device_key) {
        Some(c) => c,
        None => {
            return ToolExecutionResult {
                request_id: String::new(),
                exit_code: 1,
                output: format!("Device '{device_name}' is offline"),
            };
        }
    };

    let request_id = uuid::Uuid::new_v4().to_string();

    // Create oneshot channel for response
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .pending
        .entry(device_key.clone())
        .or_default()
        .insert(request_id.clone(), tx);

    // Send ExecuteToolRequest to device
    let req = ServerToClient::ExecuteToolRequest(ExecuteToolRequest {
        request_id: request_id.clone(),
        tool_name: tool_name.to_string(),
        arguments,
    });
    let json = serde_json::to_string(&req).unwrap();
    {
        let mut sink = conn.sink.lock().await;
        if let Err(e) = sink
            .send(axum::extract::ws::Message::Text(json.into()))
            .await
        {
            // Clean up pending
            if let Some(device_pending) = state.pending.get(&device_key) {
                device_pending.remove(&request_id);
            }
            return ToolExecutionResult {
                request_id,
                exit_code: 1,
                output: format!("Failed to send to device: {e}"),
            };
        }
    }
    drop(conn); // Release DashMap ref before awaiting

    // Await response with timeout
    match tokio::time::timeout(
        std::time::Duration::from_secs(TOOL_EXECUTION_TIMEOUT_SEC),
        rx,
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => ToolExecutionResult {
            request_id,
            exit_code: -2,
            output: format!("Device '{device_name}' disconnected during tool execution"),
        },
        Err(_) => {
            // Timeout — clean up pending
            if let Some(device_pending) = state.pending.get(&device_key) {
                device_pending.remove(&request_id);
            }
            warn!("Tool {tool_name} timed out on {device_name}");
            ToolExecutionResult {
                request_id,
                exit_code: -1,
                output: format!(
                    "Tool '{tool_name}' timed out after {TOOL_EXECUTION_TIMEOUT_SEC}s on '{device_name}'"
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, DeviceConnection};
    use std::sync::atomic::AtomicI64;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    /// Build a fake `shell` schema identical to what plexus-client advertises.
    /// Duplicated here (not imported) because plexus-server must NOT depend
    /// on plexus-client.
    fn fake_client_shell_schema() -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Execute a shell command on a client device. Runs in a bwrap jail rooted at the device's workspace_path (unless fs_policy=unrestricted). Default timeout 60s, max capped by the device's shell_timeout_max.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "device_name": { "type": "string" },
                        "command":      { "type": "string" },
                        "working_dir":  { "type": "string" },
                        "timeout":      { "type": "integer", "description": "Seconds; overrides default 60s, capped by device's shell_timeout_max." }
                    },
                    "required": ["device_name", "command"]
                }
            }
        })
    }

    /// Exercise the merge path without fabricating a real `WsSink` — we
    /// bypass the public `build_tool_schemas` entry point and inject the
    /// per-device data via the same intermediate tuple the real function
    /// uses, then assert the emitted schemas.
    #[test]
    fn build_tool_schemas_merges_shell_from_connected_clients() {
        let device_client_schemas = vec![
            ("laptop".to_string(), vec![fake_client_shell_schema()]),
            ("desktop".to_string(), vec![fake_client_shell_schema()]),
        ];

        let mut client_tool_groups: HashMap<String, (Value, Vec<String>)> = HashMap::new();
        for (device_name, device_schemas) in &device_client_schemas {
            for schema in device_schemas {
                let name = tool_name(schema).to_string();
                match client_tool_groups.get_mut(&name) {
                    Some((_existing, devices)) => devices.push(device_name.clone()),
                    None => {
                        client_tool_groups
                            .insert(name, (schema.clone(), vec![device_name.clone()]));
                    }
                }
            }
        }

        let (template, devices) = client_tool_groups.get("shell").expect("shell merged");
        let mut schema = template.clone();
        inject_device_name_enum(&mut schema, devices);

        assert_eq!(tool_name(&schema), "shell");
        let enum_values = schema
            .get("function")
            .and_then(|f| f.get("parameters"))
            .and_then(|p| p.get("properties"))
            .and_then(|props| props.get("device_name"))
            .and_then(|dn| dn.get("enum"))
            .and_then(|e| e.as_array())
            .expect("device_name enum should be present after injection");
        assert_eq!(enum_values.len(), 2);
        assert!(enum_values.contains(&serde_json::json!("laptop")));
        assert!(enum_values.contains(&serde_json::json!("desktop")));
    }

    /// End-to-end smoke test through the real `build_tool_schemas` by
    /// seeding AppState with a DeviceConnection. Requires a real `WsSink`;
    /// we build one by opening a real in-memory axum WebSocket.
    #[tokio::test]
    async fn build_tool_schemas_end_to_end_with_mock_device() {
        // Use a real axum WebSocket by spinning up a localhost server.
        let tmp = TempDir::new().expect("tempdir");
        let state = AppState::test_minimal(tmp.path());

        // Spin up a trivial axum WS server, connect a client, split the
        // server-side socket and hand its sink to DeviceConnection.
        use axum::extract::ws::WebSocketUpgrade;
        use axum::{Router, routing::get};
        use futures_util::StreamExt;
        use tokio::sync::oneshot;

        let (sink_tx, sink_rx) = oneshot::channel();
        let sink_tx_mutex = std::sync::Arc::new(std::sync::Mutex::new(Some(sink_tx)));

        let app = Router::new().route(
            "/ws",
            get({
                let sink_tx_mutex = sink_tx_mutex.clone();
                move |ws: WebSocketUpgrade| async move {
                    ws.on_upgrade(move |socket| async move {
                        let (sink, _stream) = socket.split();
                        if let Some(tx) = sink_tx_mutex.lock().unwrap().take() {
                            let _ = tx.send(sink);
                        }
                        // Keep the task alive briefly so the sink stays valid.
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    })
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("ws://{addr}/ws");
        let (_client_ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("client connect");
        let sink = sink_rx.await.expect("server sink");

        let user_id = "u1";
        let device_name = "laptop";
        let device_key = AppState::device_key(user_id, device_name);
        state.devices.insert(
            device_key.clone(),
            DeviceConnection {
                user_id: user_id.into(),
                device_name: device_name.into(),
                sink: std::sync::Arc::new(Mutex::new(sink)),
                last_seen: std::sync::Arc::new(AtomicI64::new(0)),
                tools: vec!["shell".into()],
                tool_schemas: vec![fake_client_shell_schema()],
                mcp_schemas: Vec::new(),
            },
        );
        state
            .devices_by_user
            .entry(user_id.into())
            .or_default()
            .push(device_key);

        let schemas = build_tool_schemas(&state, user_id);

        // Find the shell schema in the aggregated list.
        let shell = schemas
            .iter()
            .find(|s| tool_name(s) == "shell")
            .expect("shell should be aggregated");
        let enum_values = shell
            .get("function")
            .and_then(|f| f.get("parameters"))
            .and_then(|p| p.get("properties"))
            .and_then(|props| props.get("device_name"))
            .and_then(|dn| dn.get("enum"))
            .and_then(|e| e.as_array())
            .expect("device_name enum");
        assert!(enum_values.contains(&serde_json::json!("laptop")));
        assert!(!enum_values.contains(&serde_json::json!("server")));

        server.abort();
    }
}
