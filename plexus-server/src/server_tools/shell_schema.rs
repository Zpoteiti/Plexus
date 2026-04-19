use serde_json::{Value, json};

/// Canonical shell tool schema. `device_name` enum is injected at runtime
/// with the client devices that reported shell capability. Server is NOT
/// in the enum — server has no bwrap jail for shell.
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
