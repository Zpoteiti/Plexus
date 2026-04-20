mod config;
mod connection;
mod env;
mod guardrails;
mod heartbeat;
mod mcp;
mod read_stream;
mod sandbox;
mod tool_schemas;
mod tools;

use base64::Engine;
use connection::{WsSink, recv_message, send_message};
use heartbeat::{ack_heartbeat, spawn_heartbeat};
use plexus_common::consts::DEVICE_TOKEN_PREFIX;
use plexus_common::errors::{ErrorCode, PlexusError};
use plexus_common::protocol::{ClientToServer, ServerToClient, ToolExecutionResult};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let ws_url = std::env::var("PLEXUS_SERVER_WS_URL")
        .or_else(|_| std::env::var("PLEXUS_WS_URL"))
        .expect("PLEXUS_SERVER_WS_URL or PLEXUS_WS_URL must be set");

    let token = std::env::var("PLEXUS_AUTH_TOKEN")
        .or_else(|_| std::env::var("PLEXUS_DEVICE_TOKEN"))
        .expect("PLEXUS_AUTH_TOKEN or PLEXUS_DEVICE_TOKEN must be set");

    if !token.starts_with(DEVICE_TOKEN_PREFIX) {
        error!("Token must start with '{DEVICE_TOKEN_PREFIX}'");
        std::process::exit(1);
    }

    info!("PLEXUS Client starting...");
    reconnect_loop(&ws_url, &token).await;
}

async fn reconnect_loop(ws_url: &str, token: &str) {
    let mut backoff = 1u64;
    loop {
        match run_session(ws_url, token).await {
            Ok(()) => {
                info!("Session ended cleanly");
                backoff = 1;
            }
            Err(e) => {
                warn!("Session error: {e}");
            }
        }
        info!("Reconnecting in {backoff}s...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}

async fn run_session(ws_url: &str, token: &str) -> Result<(), PlexusError> {
    let (sink, mut stream, initial_config) = connection::connect_and_auth(ws_url, token).await?;
    let config = Arc::new(RwLock::new(initial_config));
    let sink = Arc::new(Mutex::new(sink));
    let missed_acks = Arc::new(AtomicU32::new(0));
    let dead_signal = CancellationToken::new();

    // Initialize MCP servers
    let mcp_manager = Arc::new(Mutex::new(mcp::McpManager::new()));
    {
        let cfg = config.read().await;
        mcp_manager.lock().await.initialize(&cfg.mcp_servers).await;
    }

    // Build tool registry
    let mut registry = tools::ToolRegistry::new();
    tools::register_builtin_tools(&mut registry);
    let registry = Arc::new(registry);

    // Collect and send tool names (built-in + MCP) + client-only tool schemas.
    {
        let mut tool_names = registry.tool_names();
        let mcp_names: Vec<String> = mcp_manager
            .lock()
            .await
            .all_tool_schemas()
            .into_iter()
            .filter_map(|s| {
                s.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        tool_names.extend(mcp_names);
        let msg = ClientToServer::RegisterTools {
            tool_names,
            tool_schemas: crate::tool_schemas::client_tool_schemas(),
        };
        let mut s = sink.lock().await;
        send_message(&mut s, &msg).await?;
        info!(
            "Registered {} built-in + {} MCP tools",
            registry.tool_count(),
            mcp_manager.lock().await.session_count()
        );
    }

    let hb = spawn_heartbeat(
        Arc::clone(&sink),
        Arc::clone(&missed_acks),
        dead_signal.clone(),
    );
    let stream_semaphore = read_stream::new_stream_semaphore();
    let result = message_loop(
        &mut stream,
        &sink,
        &config,
        &missed_acks,
        &registry,
        &mcp_manager,
        &dead_signal,
        &stream_semaphore,
    )
    .await;
    hb.cancel();
    result
}

#[allow(clippy::too_many_arguments)] // collaborators; session-scoped, not worth bundling
async fn message_loop(
    stream: &mut connection::WsStream,
    sink: &Arc<Mutex<WsSink>>,
    config: &Arc<RwLock<config::ClientConfig>>,
    missed_acks: &Arc<AtomicU32>,
    registry: &Arc<tools::ToolRegistry>,
    mcp_manager: &Arc<Mutex<mcp::McpManager>>,
    dead_signal: &CancellationToken,
    stream_semaphore: &Arc<tokio::sync::Semaphore>,
) -> Result<(), PlexusError> {
    loop {
        let msg = tokio::select! {
            _ = dead_signal.cancelled() => {
                return Err(PlexusError::new(ErrorCode::ConnectionFailed, "heartbeat: connection dead"));
            }
            msg = recv_message(stream) => msg?,
        };
        match msg {
            ServerToClient::HeartbeatAck => {
                ack_heartbeat(missed_acks);
            }
            ServerToClient::ExecuteToolRequest(req) => {
                let sink = Arc::clone(sink);
                let config = Arc::clone(config);
                let registry = Arc::clone(registry);
                let mcp_mgr = Arc::clone(mcp_manager);

                // Spawn tool execution in background so message loop continues
                tokio::spawn(async move {
                    let result = if mcp::McpManager::is_mcp_tool(&req.tool_name) {
                        match mcp_mgr
                            .lock()
                            .await
                            .call_tool(&req.tool_name, req.arguments)
                            .await
                        {
                            Ok(out) => tools::ToolResult::success(out),
                            Err(e) => tools::ToolResult::error(e),
                        }
                    } else {
                        let cfg = config.read().await;
                        registry.dispatch(&req.tool_name, req.arguments, &cfg).await
                    };

                    let msg = ClientToServer::ToolExecutionResult(ToolExecutionResult {
                        request_id: req.request_id,
                        exit_code: result.exit_code,
                        output: result.output,
                    });
                    if let Err(e) = send_message(&mut *sink.lock().await, &msg).await {
                        warn!("send result failed: {e}");
                    }
                });
            }
            ServerToClient::ConfigUpdate {
                fs_policy,
                mcp_servers,
                workspace_path,
                shell_timeout_max,
                ssrf_whitelist,
            } => {
                let mut cfg = config.write().await;
                let (mcp_changed, workspace_path_changed) = cfg.merge_update(
                    fs_policy,
                    mcp_servers.clone(),
                    workspace_path,
                    shell_timeout_max,
                    ssrf_whitelist,
                );
                if workspace_path_changed {
                    // TODO: trigger reconnect so bwrap jail rebinds to new workspace_path.
                    // For now, log — tools called after this point still see the new config
                    // via the Arc<RwLock<ClientConfig>>; a clean reconnect would be cleaner.
                    info!(
                        "ConfigUpdate: workspace_path changed to {:?}; applying in-place (reconnect for bwrap rebind not yet wired)",
                        cfg.workspace
                    );
                }
                drop(cfg);
                if mcp_changed && let Some(new_servers) = mcp_servers {
                    let mut mgr = mcp_manager.lock().await;
                    mgr.apply_config(&new_servers).await;
                    // Re-register tools with updated MCP names
                    let mut tool_names = registry.tool_names();
                    let mcp_names: Vec<String> = mgr
                        .all_tool_schemas()
                        .into_iter()
                        .filter_map(|s| {
                            s.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                                .map(|s| s.to_string())
                        })
                        .collect();
                    tool_names.extend(mcp_names);
                    let msg = ClientToServer::RegisterTools {
                        tool_names,
                        tool_schemas: crate::tool_schemas::client_tool_schemas(),
                    };
                    let _ = send_message(&mut *sink.lock().await, &msg).await;
                }
            }
            ServerToClient::FileRequest { request_id, path } => {
                let sink = Arc::clone(sink);
                let config = Arc::clone(config);
                tokio::spawn(async move {
                    let cfg = config.read().await;
                    let resp = match tools::helpers::sanitize_path(&path, &cfg, false) {
                        Ok(resolved) => match tokio::fs::read(&resolved).await {
                            Ok(bytes) => {
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                ClientToServer::FileResponse {
                                    request_id,
                                    content_base64: b64,
                                    mime_type: None,
                                    error: None,
                                }
                            }
                            Err(e) => ClientToServer::FileResponse {
                                request_id,
                                content_base64: String::new(),
                                mime_type: None,
                                error: Some(format!("Read failed: {e}")),
                            },
                        },
                        Err(e) => ClientToServer::FileResponse {
                            request_id,
                            content_base64: String::new(),
                            mime_type: None,
                            error: Some(e),
                        },
                    };
                    if let Err(e) = send_message(&mut *sink.lock().await, &resp).await {
                        warn!("send FileResponse failed: {e}");
                    }
                });
            }
            ServerToClient::FileSend {
                request_id,
                filename: _,
                content_base64,
                destination,
            } => {
                let sink = Arc::clone(sink);
                let config = Arc::clone(config);
                tokio::spawn(async move {
                    let cfg = config.read().await;
                    let ack_err = match tools::helpers::sanitize_path(&destination, &cfg, true) {
                        Ok(resolved) => {
                            match base64::engine::general_purpose::STANDARD.decode(&content_base64)
                            {
                                Ok(bytes) => {
                                    let mkdir_err = if let Some(parent) = resolved.parent() {
                                        tokio::fs::create_dir_all(parent)
                                            .await
                                            .err()
                                            .map(|e| format!("Create dirs failed: {e}"))
                                    } else {
                                        None
                                    };
                                    match mkdir_err {
                                        Some(e) => Some(e),
                                        None => match tokio::fs::write(&resolved, &bytes).await {
                                            Ok(()) => None,
                                            Err(e) => Some(format!("Write failed: {e}")),
                                        },
                                    }
                                }
                                Err(e) => Some(format!("Decode base64: {e}")),
                            }
                        }
                        Err(e) => Some(e),
                    };
                    let resp = ClientToServer::FileSendAck {
                        request_id,
                        error: ack_err,
                    };
                    if let Err(e) = send_message(&mut *sink.lock().await, &resp).await {
                        warn!("send FileSendAck failed: {e}");
                    }
                });
            }
            ServerToClient::ReadStream { request_id, path } => {
                // FR1b: server-initiated file streaming. Spawn so the
                // message loop isn't blocked for the duration of the
                // read. Concurrency is bounded by `stream_semaphore`
                // (MAX_CONCURRENT_STREAMS permits); saturation yields
                // an immediate StreamError to the server.
                let sink_ws: Arc<dyn read_stream::FrameSink> = Arc::new(read_stream::WsFrameSink {
                    sink: Arc::clone(sink),
                });
                let config = Arc::clone(config);
                let sem = Arc::clone(stream_semaphore);
                tokio::spawn(async move {
                    read_stream::handle(sink_ws, config, sem, request_id, path).await;
                });
            }
            other => {
                warn!("Unexpected message: {other:?}");
            }
        }
    }
}
