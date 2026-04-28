//! Hardcoded JSON schemas for the 14 first-class tools (ADR-038).
//!
//! Each schema is a `LazyLock<serde_json::Value>` parsed exactly once
//! per process via the `serde_json::json!` macro. Compile-time JSON
//! syntax check; zero runtime startup cost.

use serde_json::{Value, json};
use std::sync::LazyLock;

pub static READ_FILE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "read_file",
        "description": "Read a file (text, image, or document). Text output format: LINE_NUM|CONTENT. Images return visual content for analysis. Supports PDF, DOCX, XLSX, PPTX documents. Use offset and limit for large text files. Reads exceeding ~128K chars are truncated.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The file path to read" },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-indexed, default 1)",
                    "minimum": 1
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (default 2000)",
                    "minimum": 1
                },
                "pages": {
                    "type": "string",
                    "description": "Page range for PDF files, e.g. '1-5' (default: all, max 20 pages)"
                }
            },
            "required": ["path"]
        }
    })
});

pub static WRITE_FILE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "write_file",
        "description": "Write content to a file. Creates the file if it does not exist; overwrites if it does. Implicit mkdir -p on the parent directory.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The file path to write" },
                "content": { "type": "string", "description": "Bytes to write" }
            },
            "required": ["path", "content"]
        }
    })
});

pub static EDIT_FILE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "edit_file",
        "description": "Replace text in a file. Three-level fuzzy match: exact, whitespace-insensitive, line-based. Set replace_all=true to replace every occurrence; default false replaces the first match only.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_text": { "type": "string", "description": "Text to find" },
                "new_text": { "type": "string", "description": "Replacement text" },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence (default false)",
                    "default": false
                }
            },
            "required": ["path", "old_text", "new_text"]
        }
    })
});

pub static DELETE_FILE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "delete_file",
        "description": "Remove a single file. Always allowed (releases quota).",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        }
    })
});

pub static DELETE_FOLDER_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "delete_folder",
        "description": "Recursively remove a folder and all its contents.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        }
    })
});

pub static LIST_DIR_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "list_dir",
        "description": "List entries in a directory.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "recursive": { "type": "boolean", "default": false },
                "max_entries": { "type": "integer", "minimum": 1, "default": 1000 }
            },
            "required": ["path"]
        }
    })
});

pub static GLOB_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "glob",
        "description": "Find files matching a glob pattern (e.g. '**/*.rs'). Returns sorted list of matching paths.",
        "input_schema": {
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern" },
                "path": { "type": "string", "description": "Root directory (default: workspace root)" },
                "max_results": { "type": "integer", "minimum": 1, "default": 1000 },
                "head_limit": { "type": "integer", "minimum": 1 },
                "offset": { "type": "integer", "minimum": 0, "default": 0 },
                "entry_type": {
                    "type": "string",
                    "enum": ["file", "directory", "any"],
                    "default": "file"
                }
            },
            "required": ["pattern"]
        }
    })
});

pub static GREP_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "grep",
        "description": "Search file contents for a regex pattern. Multiple output modes (content, files_with_matches, count). Supports context lines, file-type filtering, head limit, offset for pagination.",
        "input_schema": {
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression to search for" },
                "path": { "type": "string", "description": "Directory or file to search (default: workspace root)" },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "default": "content"
                },
                "fixed_strings": { "type": "boolean", "default": false },
                "case_insensitive": { "type": "boolean", "default": false },
                "multiline": { "type": "boolean", "default": false },
                "type": { "type": "string", "description": "File-type filter (e.g. 'rust', 'python')" },
                "context_before": { "type": "integer", "minimum": 0 },
                "context_after": { "type": "integer", "minimum": 0 },
                "context": { "type": "integer", "minimum": 0, "description": "Lines of context both before and after each match" },
                "head_limit": { "type": "integer", "minimum": 1 },
                "offset": { "type": "integer", "minimum": 0, "default": 0 },
                "show_line_numbers": { "type": "boolean", "default": true }
            },
            "required": ["pattern"]
        }
    })
});

pub static NOTEBOOK_EDIT_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "notebook_edit",
        "description": "Edit a Jupyter notebook (.ipynb) cell. Three modes: replace cell at index, insert new cell after index, or delete cell at index.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "cell_index": { "type": "integer", "minimum": 0 },
                "new_source": { "type": "string", "description": "New cell source (required for replace and insert)" },
                "cell_type": {
                    "type": "string",
                    "enum": ["code", "markdown"],
                    "description": "Cell type for insert mode (default 'code')"
                },
                "edit_mode": {
                    "type": "string",
                    "enum": ["replace", "insert", "delete"],
                    "default": "replace"
                }
            },
            "required": ["path", "cell_index"]
        }
    })
});

pub static WEB_FETCH_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "web_fetch",
        "description": "Fetch a URL and extract readable content (HTML → markdown/text). Output is capped at maxChars (default 50 000). Works for most web pages and docs; may fail on login-walled or JS-heavy sites.",
        "input_schema": {
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch" },
                "extractMode": {
                    "type": "string",
                    "enum": ["markdown", "text"],
                    "default": "markdown"
                },
                "maxChars": { "type": "integer", "minimum": 100 }
            },
            "required": ["url"]
        }
    })
});

pub static MESSAGE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "message",
        "description": "Send a message (text, media, or interactive buttons) to a chat. If channel and chat_id are omitted, delivers to the current session's channel + chat_id (the default reply path). If specified, delivers cross-channel.",
        "input_schema": {
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Message text" },
                "channel": {
                    "type": "string",
                    "description": "Target channel (e.g. 'discord', 'telegram'). Optional — defaults to the current session's channel."
                },
                "chat_id": {
                    "type": "string",
                    "description": "Target chat identifier on that channel. Required if channel is set."
                },
                "media": {
                    "type": "array",
                    "description": "Workspace paths to media files to attach. Server-side workspace_fs path relative to user's workspace.",
                    "items": { "type": "string" }
                },
                "buttons": {
                    "type": "array",
                    "description": "Inline keyboard buttons (e.g. ['Yes', 'No']). When pressed, the label is sent back as a normal user message.",
                    "items": { "type": "string" }
                }
            },
            "required": ["content"]
        }
    })
});

pub static FILE_TRANSFER_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "file_transfer",
        "description": "Copy or move a file or folder between devices (server ↔ device, device ↔ device). Same-device move is an atomic rename. Folders transfer recursively.",
        "input_schema": {
            "type": "object",
            "properties": {
                "src_path": { "type": "string" },
                "dst_path": { "type": "string" },
                "mode": {
                    "type": "string",
                    "enum": ["copy", "move"],
                    "default": "copy"
                }
            },
            "required": ["src_path", "dst_path"]
        }
    })
});

pub static CRON_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "cron",
        "description": "Manage scheduled agent invocations (add, list, remove). Triggered job runs in a dedicated session that inherits the current session's channel + chat_id.",
        "input_schema": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove"]
                },
                "message": {
                    "type": "string",
                    "description": "REQUIRED when action='add'. Instruction for the agent to execute when the job triggers (e.g., 'Send a reminder to WeChat: xxx' or 'Check system status and report'). Not used for action='list' or action='remove'."
                },
                "every_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "For recurring jobs: interval in seconds. One of every_seconds, cron_expr, or at must be provided when action='add'."
                },
                "cron_expr": {
                    "type": "string",
                    "description": "For recurring jobs: standard cron expression (5 fields)."
                },
                "at": {
                    "type": "string",
                    "description": "ISO datetime for one-time execution (e.g. '2026-02-12T10:30:00'). Naive values use the tool's default timezone."
                },
                "tz": {
                    "type": "string",
                    "description": "Timezone (e.g. 'America/Los_Angeles'). Default: UTC."
                },
                "deliver": {
                    "type": "boolean",
                    "description": "Whether to deliver the execution result to the user channel (default true)",
                    "default": true
                },
                "job_id": {
                    "type": "string",
                    "description": "REQUIRED when action='remove'. The id returned by add."
                }
            },
            "required": ["action"]
        }
    })
});

pub static EXEC_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "name": "exec",
        "description": "Execute a shell command and return its output. Prefer read_file/write_file/edit_file over cat/echo/sed, and grep/glob over shell find/grep. Use -y or --yes flags to avoid interactive prompts. Output is truncated at 10 000 chars; timeout defaults to 60s.",
        "input_schema": {
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to execute" },
                "working_dir": { "type": "string", "description": "Optional working directory for the command" },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds. Increase for long-running commands like compilation or installation (default 60, max 600).",
                    "minimum": 1,
                    "maximum": 600
                }
            },
            "required": ["command"]
        }
    })
});

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: every schema must have name + description + input_schema fields.
    fn assert_well_formed(schema: &serde_json::Value, expected_name: &str) {
        assert_eq!(
            schema.get("name").and_then(|v| v.as_str()),
            Some(expected_name),
            "schema name mismatch"
        );
        assert!(
            schema.get("description").and_then(|v| v.as_str()).is_some(),
            "schema missing description"
        );
        let input_schema = schema
            .get("input_schema")
            .expect("schema missing input_schema");
        assert_eq!(
            input_schema.get("type").and_then(|v| v.as_str()),
            Some("object")
        );
    }

    #[test]
    fn read_file_schema_well_formed() {
        assert_well_formed(&READ_FILE_SCHEMA, "read_file");
    }

    #[test]
    fn write_file_schema_well_formed() {
        assert_well_formed(&WRITE_FILE_SCHEMA, "write_file");
    }

    #[test]
    fn edit_file_schema_well_formed() {
        assert_well_formed(&EDIT_FILE_SCHEMA, "edit_file");
    }

    #[test]
    fn delete_file_schema_well_formed() {
        assert_well_formed(&DELETE_FILE_SCHEMA, "delete_file");
    }

    #[test]
    fn delete_folder_schema_well_formed() {
        assert_well_formed(&DELETE_FOLDER_SCHEMA, "delete_folder");
    }

    #[test]
    fn list_dir_schema_well_formed() {
        assert_well_formed(&LIST_DIR_SCHEMA, "list_dir");
    }

    #[test]
    fn glob_schema_well_formed() {
        assert_well_formed(&GLOB_SCHEMA, "glob");
    }

    #[test]
    fn grep_schema_well_formed() {
        assert_well_formed(&GREP_SCHEMA, "grep");
    }

    #[test]
    fn notebook_edit_schema_well_formed() {
        assert_well_formed(&NOTEBOOK_EDIT_SCHEMA, "notebook_edit");
    }

    #[test]
    fn web_fetch_schema_well_formed() {
        assert_well_formed(&WEB_FETCH_SCHEMA, "web_fetch");
    }

    #[test]
    fn read_file_required_includes_path() {
        let required = READ_FILE_SCHEMA["input_schema"]["required"]
            .as_array()
            .unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    #[test]
    fn edit_file_required_includes_old_and_new_text() {
        let required = EDIT_FILE_SCHEMA["input_schema"]["required"]
            .as_array()
            .unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"path"));
        assert!(names.contains(&"old_text"));
        assert!(names.contains(&"new_text"));
    }

    #[test]
    fn message_schema_well_formed() {
        assert_well_formed(&MESSAGE_SCHEMA, "message");
    }

    #[test]
    fn file_transfer_schema_well_formed() {
        assert_well_formed(&FILE_TRANSFER_SCHEMA, "file_transfer");
    }

    #[test]
    fn cron_schema_well_formed() {
        assert_well_formed(&CRON_SCHEMA, "cron");
    }

    #[test]
    fn exec_schema_well_formed() {
        assert_well_formed(&EXEC_SCHEMA, "exec");
    }

    #[test]
    fn cron_action_enum_has_three_values() {
        let action = &CRON_SCHEMA["input_schema"]["properties"]["action"];
        let values = action["enum"].as_array().unwrap();
        let names: Vec<&str> = values.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["add", "list", "remove"]);
    }

    #[test]
    fn exec_command_required() {
        let required = EXEC_SCHEMA["input_schema"]["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
    }

    #[test]
    fn all_14_schemas_are_distinct_names() {
        let names: Vec<&str> = [
            &*READ_FILE_SCHEMA,
            &*WRITE_FILE_SCHEMA,
            &*EDIT_FILE_SCHEMA,
            &*DELETE_FILE_SCHEMA,
            &*DELETE_FOLDER_SCHEMA,
            &*LIST_DIR_SCHEMA,
            &*GLOB_SCHEMA,
            &*GREP_SCHEMA,
            &*NOTEBOOK_EDIT_SCHEMA,
            &*WEB_FETCH_SCHEMA,
            &*MESSAGE_SCHEMA,
            &*FILE_TRANSFER_SCHEMA,
            &*CRON_SCHEMA,
            &*EXEC_SCHEMA,
        ]
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
        let unique: std::collections::HashSet<_> = names.iter().copied().collect();
        assert_eq!(unique.len(), 14, "duplicate name in schemas: {:?}", names);
    }
}
