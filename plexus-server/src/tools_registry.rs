//! Tool schema merging, device_name injection, and tool call routing.
//!
//! Three categories:
//! - Native server tools (e.g. web_fetch): no device_name, emitted as-is.
//! - MCP tools (mcp_{server}_{tool}): from server MCP and/or client devices.
//!   Dedup key = full prefixed name. device_name enum = "server" + any client
//!   devices that also have it. mcp_minimax_* and mcp_anthropic_* never merged.
//! - Client native tools (e.g. read_file): device_name enum = all client devices
//!   that have the tool.

use crate::state::AppState;
use futures_util::SinkExt;
use plexus_common::consts::{SERVER_DEVICE_NAME, TOOL_EXECUTION_TIMEOUT_SEC};
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

    // 2. Collect all client device tools: (device_name, tool_schema)
    let mut device_tools: Vec<(String, Vec<Value>)> = Vec::new();
    if let Some(keys) = state.devices_by_user.get(user_id) {
        for key in keys.value() {
            if let Some(conn) = state.devices.get(key) {
                device_tools.push((conn.device_name.clone(), conn.tools.clone()));
            }
        }
    }

    // 3. Build two accumulation maps keyed by tool name:
    //    - mcp_sources: mcp_* tools → ordered list of device names (may include "server")
    //    - native_sources: non-mcp client tools → ordered list of device names
    //    - representative: first schema seen for each key (used as template)
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
        if name.is_empty() { continue; }
        mcp_sources.entry(name.clone()).or_default().push(SERVER_DEVICE_NAME.to_string());
        representatives.entry(name).or_insert(schema);
    }

    // 3b. Accumulate client tools into mcp_sources or native_sources.
    for (device_name, tools) in &device_tools {
        for schema in tools {
            let name = tool_name(schema).to_string();
            if name.is_empty() { continue; }
            if name.starts_with("mcp_") {
                mcp_sources.entry(name.clone()).or_default().push(device_name.clone());
            } else {
                native_sources.entry(name.clone()).or_default().push(device_name.clone());
            }
            representatives.entry(name).or_insert_with(|| schema.clone());
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

    // 5. Emit client native tools — one schema per unique name, device_name enum = all devices.
    let mut native_keys: Vec<String> = native_sources.keys().cloned().collect();
    native_keys.sort();
    for name in native_keys {
        let devices = &native_sources[&name];
        if let Some(template) = representatives.get(&name) {
            let mut schema = template.clone();
            inject_device_name_enum(&mut schema, devices);
            schemas.push(schema);
        }
    }

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
    if let Some(function) = schema.get_mut("function") {
        if let Some(parameters) = function.get_mut("parameters") {
            if let Some(properties) = parameters.get_mut("properties") {
                if let Some(obj) = properties.as_object_mut() {
                    obj.insert(
                        "device_name".to_string(),
                        serde_json::json!({
                            "type": "string",
                            "enum": devices,
                            "description": "Target device to execute this tool on"
                        }),
                    );
                }
            }
            // Add device_name to required
            if let Some(required) = parameters.get_mut("required") {
                if let Some(arr) = required.as_array_mut() {
                    arr.push(Value::String("device_name".into()));
                }
            }
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
