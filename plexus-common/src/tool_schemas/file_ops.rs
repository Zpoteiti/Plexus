//! Canonical JSON schemas for the 7 file tools (read_file, write_file, edit_file,
//! delete_file, list_dir, glob, grep).
//!
//! Schemas here do NOT include a `device_name` enum — that is injected at runtime
//! by `tools_registry::build_tool_schemas` based on which devices have each tool.
//! A bare `device_name: { type: string }` placeholder is present so the property
//! appears in the schema even before enum injection.

use serde_json::{Value, json};

pub fn read_file_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read a text file. When device_name is \"server\", path is relative to your server workspace. When device_name is a client device, path is that device's absolute or cwd-relative path. Returns UTF-8 content; binary or files > 256 KiB return a size hint prompting you to use file_transfer instead.",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["device_name", "path"]
            }
        }
    })
}

pub fn write_file_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "write_file",
            "description": "Write or overwrite a text file. When device_name is \"server\", path is relative to your server workspace (quota-enforced). When device_name is a client device, path is that device's path (sandbox-enforced on the client).",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": { "type": "string" },
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["device_name", "path", "content"]
            }
        }
    })
}

pub fn edit_file_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "edit_file",
            "description": "Surgical edit of a text file. Replaces exactly one occurrence of old_text with new_text. Errors if old_text is missing or appears more than once — include surrounding context to disambiguate. When device_name is \"server\", path is relative to your server workspace. When device_name is a client device, path is that device's path.",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": { "type": "string" },
                    "path": { "type": "string" },
                    "old_text": { "type": "string" },
                    "new_text": { "type": "string" }
                },
                "required": ["device_name", "path", "old_text", "new_text"]
            }
        }
    })
}

pub fn delete_file_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "delete_file",
            "description": "Delete a file or directory. Directories require recursive: true. When device_name is \"server\", path is relative to your server workspace (freed bytes credited to quota). When device_name is a client device, path is that device's path.",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": { "type": "string" },
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" }
                },
                "required": ["device_name", "path"]
            }
        }
    })
}

pub fn list_dir_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "list_dir",
            "description": "List entries (files and subdirectories) in a directory. Returns JSON rows with name, is_dir, and size_bytes. When device_name is \"server\", path is relative to your server workspace (defaults to workspace root). When device_name is a client device, path is that device's path.",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["device_name"]
            }
        }
    })
}

pub fn glob_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "glob",
            "description": "Match files against a glob pattern. Supports * (any chars except /), ** (any depth), ?, and character classes. Returns up to 500 matching paths. When device_name is \"server\", pattern is relative to workspace root. When device_name is a client device, pattern is relative to that device's cwd.",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": { "type": "string" },
                    "pattern": { "type": "string" }
                },
                "required": ["device_name", "pattern"]
            }
        }
    })
}

pub fn grep_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "grep",
            "description": "Search file content. Matches a literal substring (default) or a regular expression (regex: true). Optional path_prefix narrows the search. Returns at most 200 matching lines in `{path}:{line}: {content}` format. When device_name is \"server\", search is in your server workspace. When device_name is a client device, search is on that device.",
            "parameters": {
                "type": "object",
                "properties": {
                    "device_name": { "type": "string" },
                    "pattern": { "type": "string" },
                    "path_prefix": { "type": "string" },
                    "regex": { "type": "boolean" }
                },
                "required": ["device_name", "pattern"]
            }
        }
    })
}
