//! Server-native tool registry and dispatch.
//! Tools execute on the server, not on client devices.

use crate::state::AppState;
use plexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use std::sync::Arc;

pub mod cron_tool;
pub mod file_ops;
pub mod file_transfer;
pub mod message;
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
pub const DREAM_PHASE2_ALLOWLIST: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "delete_file",
    "list_dir",
    "glob",
    "grep",
];

/// All server tool names.
pub const SERVER_TOOL_NAMES: &[&str] = &[
    "message",
    "file_transfer",
    "cron",
    "web_fetch",
    "read_file",
    "write_file",
    "edit_file",
    "delete_file",
    "list_dir",
    "glob",
    "grep",
];

/// Check if a tool name is a server-native tool.
pub fn is_server_tool(name: &str) -> bool {
    SERVER_TOOL_NAMES.contains(&name)
}

/// Return JSON schemas for every registered server tool.
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
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a text file from your server workspace (relative path). Returns UTF-8 content for text files; for binary/non-UTF-8 files or files larger than 256 KiB, returns a size hint prompting you to use file_transfer instead.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write or overwrite a text file in your server workspace (relative path). Creates parent directories as needed. Counted against your per-user quota; rejected if the upload exceeds the per-upload cap or the workspace is soft-locked.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Surgical edit of a text file in your server workspace (relative path). Replaces exactly one occurrence of old_string with new_string. Matches are non-overlapping (e.g., 'aaa' matches once in 'aaaa'). Errors if old_string is missing or appears more than once — include surrounding context to disambiguate. Files must be ≤ 256 KiB and valid UTF-8.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old_string": { "type": "string" },
                        "new_string": { "type": "string" }
                    },
                    "required": ["path", "old_string", "new_string"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "delete_file",
                "description": "Delete a file or directory from your server workspace (relative path). Directories require recursive: true. Frees the reclaimed bytes against your quota. Refuses to delete the workspace root itself.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "recursive": { "type": "boolean" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List the entries (files and subdirectories) in a directory within your server workspace. Returns JSON rows with name, is_dir, and size_bytes. Path defaults to '.' (workspace root). Directories are listed first, then files, alphabetically within each group.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } }
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "glob",
                "description": "Match files in your server workspace against a glob pattern (relative to workspace root). Supports * (any chars except /), ** (any depth), ?, and character classes. Returns up to 500 matching relative paths, one per line. Example patterns: 'skills/*/SKILL.md', '**/*.md', 'uploads/2026-*.png'.",
                "parameters": {
                    "type": "object",
                    "properties": { "pattern": { "type": "string" } },
                    "required": ["pattern"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search file content in your server workspace. Matches either a literal substring (default) or a regular expression (regex: true). Optional path_prefix narrows the search to a subdirectory. Returns at most 200 matching lines in `{path}:{line}: {content}` format.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path_prefix": { "type": "string" },
                        "regex": { "type": "boolean" }
                    },
                    "required": ["pattern"]
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

/// Dispatch a server tool call. Returns exit_code + output.
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
        "read_file" => file_ops::read_file(state, &ctx.user_id, &arguments).await,
        "write_file" => file_ops::write_file(state, &ctx.user_id, &arguments).await,
        "edit_file" => file_ops::edit_file(state, &ctx.user_id, &arguments).await,
        "delete_file" => file_ops::delete_file(state, &ctx.user_id, &arguments).await,
        "list_dir" => file_ops::list_dir(state, &ctx.user_id, &arguments).await,
        "glob" => file_ops::glob(state, &ctx.user_id, &arguments).await,
        "grep" => file_ops::grep(state, &ctx.user_id, &arguments).await,
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
    fn dream_phase2_allowlist_covers_all_file_tools_in_registry() {
        // Sanity: every name in DREAM_PHASE2_ALLOWLIST is a registered
        // server tool (either in SERVER_TOOL_NAMES or implemented as a
        // server tool somewhere in the codebase). If we ever rename a
        // file tool, this test fires.
        for name in DREAM_PHASE2_ALLOWLIST {
            assert!(
                SERVER_TOOL_NAMES.contains(name),
                "DREAM_PHASE2_ALLOWLIST contains '{name}' which is not in SERVER_TOOL_NAMES"
            );
        }
    }
}
