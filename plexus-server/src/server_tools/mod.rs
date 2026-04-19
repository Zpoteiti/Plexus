//! Server-native tool registry and dispatch.
//! Tools execute on the server, not on client devices.

use crate::state::AppState;
use plexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use std::sync::Arc;

pub mod cron_tool;
pub mod dispatch;
pub mod file_ops;
pub mod file_ops_schemas;
pub mod file_transfer;
pub mod message;
pub mod shell_schema;
pub mod web_fetch;

/// Allowlist for tool dispatch. Used by restricted modes (e.g. dream phase 2)
/// to forbid tools outside a small set without touching the global registry.
#[derive(Debug, Clone)]
pub enum ToolAllowlist {
    /// Every registered tool is dispatchable.
    All,
    /// Only tools whose names appear in the slice may dispatch.
    Only(&'static [&'static str]),
}

impl ToolAllowlist {
    pub fn allows(&self, tool_name: &str) -> bool {
        match self {
            ToolAllowlist::All => true,
            ToolAllowlist::Only(names) => names.contains(&tool_name),
        }
    }
}

/// Tools available during dream Phase 2: file I/O only. No message, cron,
/// file_transfer, or web_fetch — dream is silent and workspace-local.
/// Dream agent MUST pass `device_name="server"` for all of these (post-unification
/// they route through dispatch::dispatch_file_tool, not server_tools::execute).
pub const DREAM_PHASE2_ALLOWLIST: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "delete_file",
    "list_dir",
    "glob",
    "grep",
];

/// Server-only tool names (message, file_transfer, cron, web_fetch).
/// File tools (read_file/write_file/etc.) are no longer listed here —
/// they are dispatched via `dispatch::dispatch_file_tool` with a unified
/// `device_name` enum covering "server" + all applicable client devices.
pub const SERVER_TOOL_NAMES: &[&str] = &["message", "file_transfer", "cron", "web_fetch"];

/// Check if a tool name is a server-native tool.
pub fn is_server_tool(name: &str) -> bool {
    SERVER_TOOL_NAMES.contains(&name)
}

/// Return JSON schemas for the 4 server-only tools (message, file_transfer, cron, web_fetch).
/// File tool schemas (read_file/write_file/etc.) are emitted by
/// `tools_registry::build_tool_schemas` with a unified `device_name` enum.
pub fn tool_schemas() -> Vec<Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "message",
                "description": "Send a message to a channel (gateway or discord), optionally with media files. When `from_device` is \"server\", media paths are relative to your server workspace (e.g. 'uploads/report.pdf'). When `from_device` is a client device name, media paths are that device's paths.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string" },
                        "channel": { "type": "string", "enum": ["gateway", "discord"] },
                        "chat_id": { "type": "string" },
                        "media": { "type": "array", "items": { "type": "string" } },
                        "from_device": { "type": "string" }
                    },
                    "required": ["content", "channel"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "file_transfer",
                "description": "Transfer a file between devices. When a side is \"server\", file_path is relative to your server workspace (e.g. 'uploads/report.pdf', 'skills/git/SKILL.md'). When a side is a client device, file_path is that device's absolute or cwd-relative path. Server-landed files always go to 'uploads/{basename(file_path)}' — rename afterwards with write_file+delete_file if you want a different location.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from_device": { "type": "string" },
                        "to_device": { "type": "string" },
                        "file_path": { "type": "string" }
                    },
                    "required": ["from_device", "to_device", "file_path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cron",
                "description": "Manage scheduled tasks: add, list, or remove cron jobs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["add", "list", "remove"] },
                        "name": { "type": "string" },
                        "message": { "type": "string" },
                        "cron_expr": { "type": "string" },
                        "every_seconds": { "type": "integer" },
                        "at": { "type": "string" },
                        "timezone": { "type": "string" },
                        "channel": { "type": "string" },
                        "chat_id": { "type": "string" },
                        "delete_after_run": { "type": "boolean" },
                        "deliver": { "type": "boolean" },
                        "job_id": { "type": "string" }
                    },
                    "required": ["action"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a URL and return readable content. SSRF-protected.",
                "parameters": {
                    "type": "object",
                    "properties": { "url": { "type": "string" } },
                    "required": ["url"]
                }
            }
        }),
    ]
}

/// Session context passed to server tools for channel/chat routing.
pub struct ToolContext {
    pub user_id: String,
    pub session_id: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub is_cron: bool,
    /// True if the sender is the owner/partner of this channel (or if this
    /// is a cron/server-originated event with no identity). False for
    /// allow-listed non-partner senders on Discord/Telegram.
    pub is_partner: bool,
}

/// Dispatch a server-only tool call (message, file_transfer, cron, web_fetch).
/// File tools (read_file/write_file/etc.) are NOT dispatched here — they go
/// through `dispatch::dispatch_file_tool` in the agent loop.
pub async fn execute(
    state: &Arc<AppState>,
    ctx: &ToolContext,
    tool_name: &str,
    arguments: Value,
) -> ToolExecutionResult {
    let request_id = uuid::Uuid::new_v4().to_string();
    let (exit_code, output) = match tool_name {
        "web_fetch" => web_fetch::web_fetch(state, &ctx.user_id, &arguments).await,
        "message" => message::message_tool(state, ctx, &arguments).await,
        "file_transfer" => file_transfer::file_transfer(state, &ctx.user_id, &arguments).await,
        "cron" => cron_tool::cron(state, ctx, &arguments).await,
        _ => (1, format!("Unknown server tool: {tool_name}")),
    };
    ToolExecutionResult {
        request_id,
        exit_code,
        output,
    }
}

#[cfg(test)]
mod allowlist_tests {
    use super::*;

    #[test]
    fn all_allows_every_tool_name() {
        let a = ToolAllowlist::All;
        assert!(a.allows("read_file"));
        assert!(a.allows("message"));
        assert!(a.allows("cron"));
        assert!(a.allows("anything_else"));
    }

    #[test]
    fn only_permits_named_tools_rejects_others() {
        let a = ToolAllowlist::Only(DREAM_PHASE2_ALLOWLIST);
        // Allowed — all 7 file tools.
        assert!(a.allows("read_file"));
        assert!(a.allows("write_file"));
        assert!(a.allows("edit_file"));
        assert!(a.allows("delete_file"));
        assert!(a.allows("list_dir"));
        assert!(a.allows("glob"));
        assert!(a.allows("grep"));
        // Rejected — non-file tools.
        assert!(!a.allows("message"));
        assert!(!a.allows("cron"));
        assert!(!a.allows("web_fetch"));
        assert!(!a.allows("file_transfer"));
        assert!(!a.allows("nonexistent"));
    }

    #[test]
    fn dream_phase2_allowlist_covers_all_file_tools_in_dispatch() {
        // Sanity: every name in DREAM_PHASE2_ALLOWLIST is a recognized
        // file tool in dispatch::FILE_TOOL_NAMES. If we rename a file tool,
        // this test fires (post-unification, file tools are no longer in
        // SERVER_TOOL_NAMES — they route through dispatch::dispatch_file_tool).
        for name in DREAM_PHASE2_ALLOWLIST {
            assert!(
                crate::server_tools::dispatch::FILE_TOOL_NAMES.contains(name),
                "DREAM_PHASE2_ALLOWLIST contains '{name}' which is not in dispatch::FILE_TOOL_NAMES"
            );
        }
    }
}
