//! MCP tool wrapping + schema-collision detection.
//!
//! `wrap_mcp_tool` takes a raw MCP tool schema and prefixes the name with
//! `mcp_<server>_` + injects a `device_name` enum into the parameters.
//!
//! `check_mcp_schema_collision` detects when two install sites advertise
//! tools with the same prefixed name but different schemas — that would
//! make the LLM-facing schema ambiguous.

use plexus_common::errors::mcp::McpError;
use serde_json::{Value, json};

/// Wrap a raw MCP tool schema. Output shape: one OpenAI-style function schema
/// with `name = "mcp_{server}_{tool}"` + `device_name` in parameters.
pub fn wrap_mcp_tool(mcp_server_name: &str, raw_schema: &Value, install_sites: &[String]) -> Value {
    let tool_name = raw_schema
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("unknown");
    let description = raw_schema
        .get("description")
        .cloned()
        .unwrap_or(Value::Null);
    let raw_params = raw_schema
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object", "properties": {}, "required": [] }));

    let mut params = raw_params;
    // Inject device_name into properties.
    if let Some(props) = params.get_mut("properties").and_then(|p| p.as_object_mut()) {
        props.insert(
            "device_name".into(),
            json!({
                "type": "string",
                "enum": install_sites,
                "description": "Where this MCP tool should run"
            }),
        );
    }
    // Add device_name to required.
    if let Some(req) = params.get_mut("required").and_then(|r| r.as_array_mut()) {
        req.push(json!("device_name"));
    } else {
        params["required"] = json!(["device_name"]);
    }

    json!({
        "type": "function",
        "function": {
            "name": format!("mcp_{mcp_server_name}_{tool_name}"),
            "description": description,
            "parameters": params,
        }
    })
}

/// A single MCP install (server MCP or per-device MCP).
#[derive(Debug, Clone)]
pub struct McpInstall {
    pub install_site: String,        // "server" or device_name
    pub mcp_server_name: String,     // e.g. "git"
    pub tools: Vec<(String, Value)>, // (tool_name, raw_schema)
}

/// Detect schema collisions when `incoming` is about to be added alongside `existing` installs.
///
/// A collision is: the same `mcp_server_name` appears in `incoming` and some `existing` install,
/// AND the same `tool_name` appears in both, AND the two schemas differ byte-for-byte.
/// (Different `mcp_server_name`s never collide because the prefix disambiguates.)
pub fn check_mcp_schema_collision(
    existing: &[McpInstall],
    incoming: &McpInstall,
) -> Result<(), McpError> {
    for e in existing {
        if e.mcp_server_name != incoming.mcp_server_name {
            continue;
        }
        for (tool_name, incoming_schema) in &incoming.tools {
            for (existing_tool, existing_schema) in &e.tools {
                if existing_tool == tool_name && existing_schema != incoming_schema {
                    return Err(McpError::SchemaCollision {
                        server: incoming.mcp_server_name.clone(),
                        tool: tool_name.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wrap_prefixes_name_and_injects_device_name() {
        let raw = json!({
            "name": "status",
            "description": "Git repo status",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        });
        let wrapped = wrap_mcp_tool("git", &raw, &["server".into(), "devbox".into()]);
        assert_eq!(
            wrapped["function"]["name"].as_str().unwrap(),
            "mcp_git_status"
        );
        let props = wrapped["function"]["parameters"]["properties"]
            .as_object()
            .unwrap();
        assert!(props.contains_key("device_name"));
        assert_eq!(props["device_name"]["enum"], json!(["server", "devbox"]));
        let req = wrapped["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(req.iter().any(|v| v == "device_name"));
        assert!(req.iter().any(|v| v == "path"));
    }

    #[test]
    fn collision_detected_on_schema_mismatch() {
        let existing = vec![McpInstall {
            install_site: "server".into(),
            mcp_server_name: "git".into(),
            tools: vec![("status".into(), json!({"input": "v1"}))],
        }];
        let incoming = McpInstall {
            install_site: "devbox".into(),
            mcp_server_name: "git".into(),
            tools: vec![("status".into(), json!({"input": "v2"}))],
        };
        let r = check_mcp_schema_collision(&existing, &incoming);
        assert!(matches!(r, Err(McpError::SchemaCollision { .. })));
    }

    #[test]
    fn no_collision_when_schemas_match() {
        let shared = json!({"input": "same"});
        let existing = vec![McpInstall {
            install_site: "server".into(),
            mcp_server_name: "git".into(),
            tools: vec![("status".into(), shared.clone())],
        }];
        let incoming = McpInstall {
            install_site: "devbox".into(),
            mcp_server_name: "git".into(),
            tools: vec![("status".into(), shared)],
        };
        assert!(check_mcp_schema_collision(&existing, &incoming).is_ok());
    }

    #[test]
    fn no_collision_when_mcp_server_names_differ() {
        let existing = vec![McpInstall {
            install_site: "server".into(),
            mcp_server_name: "git".into(),
            tools: vec![("status".into(), json!({"x": 1}))],
        }];
        let incoming = McpInstall {
            install_site: "devbox".into(),
            mcp_server_name: "fs".into(),
            tools: vec![("status".into(), json!({"x": 2}))],
        };
        assert!(check_mcp_schema_collision(&existing, &incoming).is_ok());
    }
}
