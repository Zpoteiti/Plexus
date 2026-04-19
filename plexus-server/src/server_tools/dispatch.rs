//! File-tool dispatch: unified `device_name` routing.
//!
//! All 7 file tools (read_file, write_file, edit_file, delete_file, list_dir,
//! glob, grep) are routed through here. The agent loop checks `is_file_tool`
//! BEFORE checking `is_server_tool`, so file tools never reach server_tools::execute.

use crate::state::AppState;
use plexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use std::sync::Arc;

/// Canonical set of file tools that can run on either server or a client device.
/// Note: `shell` is NOT here — shell runs only on clients (server has no bwrap jail).
pub const FILE_TOOL_NAMES: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "delete_file",
    "list_dir",
    "glob",
    "grep",
];

pub fn is_file_tool(name: &str) -> bool {
    FILE_TOOL_NAMES.contains(&name)
}

/// Dispatch a file tool call routed by `device_name`.
/// `"server"` → route to server file_ops (workspace_fs).
/// anything else → route to client via `tools_registry::route_to_device`.
pub async fn dispatch_file_tool(
    state: &Arc<AppState>,
    user_id: &str,
    tool_name: &str,
    mut args: Value,
) -> ToolExecutionResult {
    let device_name = args
        .get("device_name")
        .and_then(|v| v.as_str())
        .map(String::from);
    if let Some(obj) = args.as_object_mut() {
        obj.remove("device_name");
    }
    match device_name.as_deref() {
        Some("server") => run_on_server(state, user_id, tool_name, args).await,
        Some(d) => {
            crate::tools_registry::route_to_device(state, user_id, d, tool_name, args).await
        }
        None => ToolExecutionResult {
            request_id: String::new(),
            exit_code: 1,
            output: format!("Missing `device_name` for tool '{tool_name}'"),
        },
    }
}

async fn run_on_server(
    state: &Arc<AppState>,
    user_id: &str,
    tool_name: &str,
    args: Value,
) -> ToolExecutionResult {
    let (exit_code, output) = match tool_name {
        "read_file" => crate::server_tools::file_ops::read_file(state, user_id, &args).await,
        "write_file" => crate::server_tools::file_ops::write_file(state, user_id, &args).await,
        "edit_file" => crate::server_tools::file_ops::edit_file(state, user_id, &args).await,
        "delete_file" => crate::server_tools::file_ops::delete_file(state, user_id, &args).await,
        "list_dir" => crate::server_tools::file_ops::list_dir(state, user_id, &args).await,
        "glob" => crate::server_tools::file_ops::glob(state, user_id, &args).await,
        "grep" => crate::server_tools::file_ops::grep(state, user_id, &args).await,
        other => (1, format!("dispatch: unknown file tool '{other}'")),
    };
    ToolExecutionResult {
        request_id: String::new(),
        exit_code,
        output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn is_file_tool_rejects_non_file_tools() {
        assert!(!is_file_tool("shell"), "shell must not be a file tool");
        assert!(!is_file_tool("message"), "message must not be a file tool");
        assert!(!is_file_tool("web_fetch"), "web_fetch must not be a file tool");
        assert!(!is_file_tool("cron"), "cron must not be a file tool");
        // File tools must be recognized.
        assert!(is_file_tool("read_file"));
        assert!(is_file_tool("write_file"));
        assert!(is_file_tool("edit_file"));
        assert!(is_file_tool("delete_file"));
        assert!(is_file_tool("list_dir"));
        assert!(is_file_tool("glob"));
        assert!(is_file_tool("grep"));
    }

    #[tokio::test]
    async fn dispatch_missing_device_name_errors() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        // No device_name — should fail with exit_code=1.
        let result =
            dispatch_file_tool(&state, "alice", "read_file", json!({"path": "a.txt"})).await;
        assert_eq!(result.exit_code, 1);
        assert!(
            result.output.contains("Missing `device_name`"),
            "expected 'Missing `device_name`' in: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn dispatch_server_device_routes_to_file_ops() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        // Write a file directly to the user workspace.
        tokio::fs::write(user_dir.join("a.txt"), b"hello from dispatch")
            .await
            .unwrap();

        let state = AppState::test_minimal(tmp.path());
        let result = dispatch_file_tool(
            &state,
            "alice",
            "read_file",
            json!({"device_name": "server", "path": "a.txt"}),
        )
        .await;
        assert_eq!(
            result.exit_code, 0,
            "expected success, got: {}",
            result.output
        );
        assert_eq!(result.output, "hello from dispatch");
    }
}
