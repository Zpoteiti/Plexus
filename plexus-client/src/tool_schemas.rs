//! Tool schemas that plexus-client advertises to the server at login.
//! Only client-only tools belong here. File tools use the shared
//! plexus_common::tool_schemas::file_ops; they're not re-emitted by
//! the client since the schema is canonical in common.
//!
//! The schema returned here is canonical for the shell tool — server
//! does NOT hold a copy. `device_name` enum is injected at runtime by
//! `plexus-server::tools_registry::build_tool_schemas` with the client
//! devices that reported shell capability; server is never in the enum
//! (no bwrap jail for shell on the server).

use serde_json::{Value, json};

/// Canonical shell tool schema. Advertised by every connected client at
/// login time via `ClientToServer::RegisterTools::tool_schemas`.
pub fn shell_schema() -> Value {
    json!({
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

/// All client-only tool schemas advertised to the server at login.
/// File tools are NOT re-emitted here — the server owns their canonical
/// schemas via `plexus_common::file_ops_schemas` and routes them either
/// to the server workspace or to the client that reported the capability.
pub fn client_tool_schemas() -> Vec<Value> {
    vec![shell_schema()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snapshot test: the schema bytes here MUST match what the
    /// now-deleted `plexus-server/src/server_tools/shell_schema.rs`
    /// emitted before BF5. If you're tempted to change this JSON,
    /// first ask whether the aggregated tool list that the LLM sees
    /// should truly change — and update the snapshot intentionally.
    #[test]
    fn shell_schema_matches_pre_move_snapshot() {
        let schema = shell_schema();
        let expected = json!({
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
        });
        assert_eq!(schema, expected);
    }

    #[test]
    fn client_tool_schemas_contains_shell() {
        let schemas = client_tool_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(
            schemas[0]
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str()),
            Some("shell")
        );
    }
}
