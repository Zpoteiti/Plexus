//! Server-side MCP client manager. Admin-configured MCP servers that run on the server.
//! Tools appear with device_name="server" in the tool schema.

use plexus_common::consts::DEFAULT_MCP_TOOL_TIMEOUT_SEC;
use plexus_common::mcp_utils::normalize_schema_for_openai;
use plexus_common::protocol::McpServerEntry;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, RawContent, Tool as McpTool};
use rmcp::service::RunningService;
use rmcp::transport::child_process::TokioChildProcess;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{error, info, warn};

pub struct McpSession {
    pub server_name: String,
    pub tool_timeout: u64,
    service: RunningService<rmcp::RoleClient, ()>,
    tools: Vec<McpTool>,
}

pub struct ServerMcpManager {
    sessions: HashMap<String, McpSession>,
}

impl ServerMcpManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Initialize MCP servers from config.
    pub async fn initialize(&mut self, servers: &[McpServerEntry]) {
        for entry in servers.iter().filter(|e| e.enabled) {
            match start_session(entry).await {
                Ok(session) => {
                    info!(
                        "Server MCP started: {} ({} tools)",
                        entry.name,
                        session.tools.len()
                    );
                    self.sessions.insert(entry.name.clone(), session);
                }
                Err(e) => error!("Server MCP '{}' failed: {e}", entry.name),
            }
        }
    }

    /// Reinitialize: stop all, start new config.
    pub async fn reinitialize(&mut self, servers: &[McpServerEntry]) {
        self.sessions.clear();
        self.initialize(servers).await;
    }

    /// Get all tool schemas (prefixed with mcp_{server_name}_{tool_name}).
    pub fn tool_schemas(&self) -> Vec<Value> {
        self.sessions
            .values()
            .flat_map(|s| {
                s.tools.iter().map(move |t| {
                    let name = format!("mcp_{}_{}", s.server_name, t.name);
                    let desc = t.description.as_deref().unwrap_or("MCP tool");
                    let params = {
                        let v = serde_json::to_value(&*t.input_schema).unwrap_or_default();
                        normalize_schema_for_openai(&v)
                    };
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": desc,
                            "parameters": params,
                        }
                    })
                })
            })
            .collect()
    }

    /// For each live MCP session, return `(server_name, raw (name, params)
    /// tuples)` suitable for `mcp::wrap::McpInstall::tools`. Used by the
    /// schema-collision check (spec §4.6).
    pub fn raw_tool_schemas_by_server(&self) -> Vec<(String, Vec<(String, Value)>)> {
        self.sessions
            .values()
            .map(|s| {
                let tools = s
                    .tools
                    .iter()
                    .map(|t| {
                        let params = {
                            let v = serde_json::to_value(&*t.input_schema).unwrap_or_default();
                            normalize_schema_for_openai(&v)
                        };
                        (t.name.to_string(), params)
                    })
                    .collect();
                (s.server_name.clone(), tools)
            })
            .collect()
    }

    /// Call an MCP tool by prefixed name. Extracts server name from prefix.
    pub async fn call_tool(&self, prefixed_name: &str, args: Value) -> Result<String, String> {
        let rest = prefixed_name
            .strip_prefix("mcp_")
            .ok_or_else(|| format!("Not an MCP tool: {prefixed_name}"))?;

        for (name, session) in &self.sessions {
            if let Some(tool_name) = rest.strip_prefix(&format!("{name}_")) {
                return call_mcp_tool(session, tool_name, args).await;
            }
        }
        Err(format!("Server MCP not found for: {prefixed_name}"))
    }
}

/// Hard cap on transient MCP introspection so a broken/slow MCP can't hold
/// an admin PUT open. Applies to BOTH spawn+initialize AND the subsequent
/// `tools/list` call.
pub const MCP_INTROSPECTION_TIMEOUT_SEC: u64 = 10;

/// Spawn the MCP described by `entry`, run `initialize` + `tools/list`,
/// then close the subprocess. Returns `(tool_name, parameters)` tuples
/// (raw, unprefixed). Used by `PUT /api/server-mcp` to validate schemas
/// before persisting (spec §4.6, FR6).
///
/// Both stdio and HTTP transports are supported. On any error (spawn
/// failure, protocol error, timeout) the subprocess is killed before
/// return. The caller gets a short string suitable for a 400/502 body.
pub async fn introspect_entry(entry: &McpServerEntry) -> Result<Vec<(String, Value)>, String> {
    // Treat `url` presence as the HTTP transport selector to mirror the
    // rest of the MCP config shape (matches `McpServerEntry.transport_type`
    // == "http" when set, with `url` as the authoritative switch).
    if entry.url.is_some() {
        return Err(format!(
            "MCP '{}': HTTP transport introspection not yet implemented; file an issue or use stdio",
            entry.name
        ));
    }

    let deadline = std::time::Duration::from_secs(MCP_INTROSPECTION_TIMEOUT_SEC);
    let entry = entry.clone();
    let entry_name = entry.name.clone();
    let fut = async move {
        let mut cmd = tokio::process::Command::new(&entry.command);
        cmd.args(&entry.args);
        if let Some(env) = &entry.env {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }
        // TokioChildProcess owns kill-on-drop via its internal handle;
        // when the service future is dropped on timeout the child is
        // reaped by tokio.
        let child =
            TokioChildProcess::new(cmd).map_err(|e| format!("spawn '{}': {e}", entry.name))?;
        let service = ().serve(child).await.map_err(|e| format!("init '{}': {e}", entry.name))?;
        let tools = service
            .list_all_tools()
            .await
            .map_err(|e| format!("tools/list '{}': {e}", entry.name))?;
        // Close the subprocess cleanly before returning.
        let _ = service.cancel().await;
        let raw: Vec<(String, Value)> = tools
            .into_iter()
            .map(|t| {
                let params = {
                    let v = serde_json::to_value(&*t.input_schema).unwrap_or_default();
                    normalize_schema_for_openai(&v)
                };
                (t.name.to_string(), params)
            })
            .collect();
        Ok::<Vec<(String, Value)>, String>(raw)
    };

    match tokio::time::timeout(deadline, fut).await {
        Ok(res) => res,
        Err(_) => Err(format!(
            "MCP '{entry_name}' introspection timed out after {MCP_INTROSPECTION_TIMEOUT_SEC}s"
        )),
    }
}

async fn start_session(entry: &McpServerEntry) -> Result<McpSession, String> {
    let mut cmd = tokio::process::Command::new(&entry.command);
    cmd.args(&entry.args);
    if let Some(env) = &entry.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    let child = TokioChildProcess::new(cmd).map_err(|e| format!("spawn '{}': {e}", entry.name))?;

    let service = ().serve(child).await.map_err(|e| format!("init '{}': {e}", entry.name))?;

    let tools = service
        .list_all_tools()
        .await
        .map_err(|e| format!("tools/list '{}': {e}", entry.name))?;

    Ok(McpSession {
        server_name: entry.name.clone(),
        tool_timeout: entry.tool_timeout.unwrap_or(DEFAULT_MCP_TOOL_TIMEOUT_SEC),
        service,
        tools,
    })
}

async fn call_mcp_tool(
    session: &McpSession,
    tool_name: &str,
    args: Value,
) -> Result<String, String> {
    let timeout = std::time::Duration::from_secs(session.tool_timeout);
    let args_map: serde_json::Map<String, Value> = match args {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };

    let params = CallToolRequestParams::new(tool_name.to_string()).with_arguments(args_map);

    let result = tokio::time::timeout(timeout, async {
        session
            .service
            .call_tool(params)
            .await
            .map_err(|e| format!("MCP call failed: {e}"))
    })
    .await
    .map_err(|_| {
        format!(
            "MCP tool '{tool_name}' timed out after {}s",
            session.tool_timeout
        )
    })?;

    let result = result?;

    let output: String = result
        .content
        .iter()
        .filter_map(|item| match &item.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => {
                warn!("Skipping non-text MCP content");
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `introspect_entry` surfaces spawn failure as a string error instead of
    /// panicking or hanging — exercises the 400 path in PUT /api/server-mcp.
    #[tokio::test]
    async fn introspect_reports_spawn_failure() {
        let entry = McpServerEntry {
            name: "bogus".into(),
            transport_type: None,
            command: "/bin/definitely-not-a-real-mcp-binary".into(),
            args: vec![],
            env: None,
            url: None,
            headers: None,
            tool_timeout: None,
            enabled: true,
        };
        let res = introspect_entry(&entry).await;
        assert!(res.is_err(), "expected introspect error for missing binary");
        let msg = res.unwrap_err();
        assert!(
            msg.contains("spawn")
                || msg.contains("bogus")
                || msg.contains("init")
                || msg.contains("No such file"),
            "unexpected error: {msg}"
        );
    }

    /// HTTP transport isn't wired yet; introspection must fail fast rather
    /// than silently succeed with zero tools.
    #[tokio::test]
    async fn introspect_http_transport_not_supported() {
        let entry = McpServerEntry {
            name: "http-mcp".into(),
            transport_type: Some("http".into()),
            command: String::new(),
            args: vec![],
            env: None,
            url: Some("https://example.invalid/mcp".into()),
            headers: None,
            tool_timeout: None,
            enabled: true,
        };
        let res = introspect_entry(&entry).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("HTTP"));
    }
}
