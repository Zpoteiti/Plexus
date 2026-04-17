//! Server-native tool registry and dispatch.
//! Tools execute on the server, not on client devices.

use crate::state::AppState;
use plexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use std::sync::Arc;

pub mod cron_tool;
pub mod file_ops;
pub mod file_transfer;
pub mod memory;
pub mod message;
pub mod skills;
pub mod web_fetch;

/// All server tool names.
pub const SERVER_TOOL_NAMES: &[&str] = &[
    "save_memory",
    "edit_memory",
    "message",
    "file_transfer",
    "cron",
    "read_skill",
    "install_skill",
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
                "name": "save_memory",
                "description": "Save text to persistent memory (replaces current memory). 4K char max.",
                "parameters": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_memory",
                "description": "Edit persistent memory: append, prepend, or replace content.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "operation": { "type": "string", "enum": ["append", "prepend", "replace"] },
                        "text": { "type": "string" }
                    },
                    "required": ["operation", "text"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "message",
                "description": "Send a message to a channel (gateway or discord), optionally with media files from a device.",
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
                "name": "read_skill",
                "description": "Read the full instructions of an on-demand skill.",
                "parameters": {
                    "type": "object",
                    "properties": { "skill_name": { "type": "string" } },
                    "required": ["skill_name"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "install_skill",
                "description": "Install a skill from a GitHub repo (fetches SKILL.md).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "repo": { "type": "string", "description": "owner/repo" },
                        "branch": { "type": "string" }
                    },
                    "required": ["repo"]
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
        "save_memory" => memory::save_memory(state, &ctx.user_id, &arguments).await,
        "edit_memory" => memory::edit_memory(state, &ctx.user_id, &arguments).await,
        "web_fetch" => web_fetch::web_fetch(state, &ctx.user_id, &arguments).await,
        "message" => message::message_tool(state, ctx, &arguments).await,
        "file_transfer" => file_transfer::file_transfer(state, &ctx.user_id, &arguments).await,
        "cron" => cron_tool::cron(state, ctx, &arguments).await,
        "read_skill" => skills::read_skill(state, &ctx.user_id, &arguments).await,
        "install_skill" => skills::install_skill(state, &ctx.user_id, &arguments).await,
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
