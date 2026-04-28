//! Frame inner types — the data shapes carried by frames in `frames.rs`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Filesystem policy controlling both the file-tool jail and the subprocess
/// jail (ADR-073).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FsPolicy {
    /// Default — file tools enforce the workspace boundary; on Linux the
    /// subprocess jail (bwrap) also fires.
    Sandbox,
    /// Both jails lifted. Requires typed-name confirmation per ADR-051.
    Unrestricted,
}

/// Device configuration sent in `hello_ack` and `config_update` (ADR-050).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Absolute workspace root path on the device.
    pub workspace_path: String,

    pub fs_policy: FsPolicy,

    /// Maximum `exec` timeout the agent can request, in seconds.
    pub shell_timeout_max: u32,

    /// Per-device SSRF whitelist for `web_fetch` (ADR-052). `host` or
    /// `host:port` strings.
    #[serde(default)]
    pub ssrf_whitelist: Vec<String>,

    /// MCP server configurations keyed by server name.
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

/// Per-MCP-server configuration (ADR-050, ADR-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Argv to spawn the subprocess (e.g. `["npx", "@plexus/mcp-google"]`).
    pub command: Vec<String>,

    /// Environment variables for the subprocess. Values may include secrets.
    #[serde(default)]
    pub env: HashMap<String, String>,

    #[serde(default)]
    pub description: Option<String>,

    /// Optional allow-list of post-wrap entry names (ADR-100). Glob patterns.
    /// When `None`, every advertised capability registers.
    #[serde(default)]
    pub enabled: Option<Vec<String>>,
}

/// All capabilities advertised by one MCP server (ADR-047, ADR-048).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpSchemas {
    pub server_name: String,

    #[serde(default)]
    pub tools: Vec<ToolDef>,

    #[serde(default)]
    pub resources: Vec<ResourceDef>,

    #[serde(default)]
    pub prompts: Vec<PromptDef>,
}

/// One tool advertised by an MCP server (raw shape from `list_tools`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,

    pub input_schema: serde_json::Value,

    #[serde(default)]
    pub description: Option<String>,
}

/// One resource advertised by an MCP server (raw shape from `list_resources`).
///
/// `uri` may be a static URI or a URI template with `{var}` placeholders
/// (ADR-099). The wrap step (Plan 3) converts the template into schema
/// properties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDef {
    pub name: String,

    pub uri: String,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default, rename = "mimeType")]
    pub mime_type: Option<String>,
}

/// One prompt advertised by an MCP server (raw shape from `list_prompts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptDef {
    pub name: String,

    #[serde(default)]
    pub arguments: Vec<PromptArgument>,

    #[serde(default)]
    pub description: Option<String>,
}

/// One argument of an MCP prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub required: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn device_config_roundtrip() {
        let cfg = DeviceConfig {
            workspace_path: "/home/alice/.plexus".into(),
            fs_policy: FsPolicy::Sandbox,
            shell_timeout_max: 300,
            ssrf_whitelist: vec!["10.180.20.30:8080".into()],
            mcp_servers: HashMap::new(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: DeviceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn fs_policy_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&FsPolicy::Sandbox).unwrap(),
            "\"sandbox\""
        );
        assert_eq!(
            serde_json::to_string(&FsPolicy::Unrestricted).unwrap(),
            "\"unrestricted\""
        );
    }

    #[test]
    fn mcp_server_config_roundtrip() {
        let cfg = McpServerConfig {
            command: vec!["npx".into(), "@plexus/mcp-google".into()],
            env: HashMap::from([("GOOGLE_API_KEY".to_string(), "redacted".to_string())]),
            description: Some("Google search".into()),
            enabled: Some(vec!["mcp_google_*".into()]),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn device_config_with_mcp_servers_roundtrip() {
        let mut servers = HashMap::new();
        servers.insert(
            "google".to_string(),
            McpServerConfig {
                command: vec!["npx".into(), "@plexus/mcp-google".into()],
                env: HashMap::from([("GOOGLE_API_KEY".to_string(), "x".to_string())]),
                description: None,
                enabled: None,
            },
        );
        let cfg = DeviceConfig {
            workspace_path: "/ws".into(),
            fs_policy: FsPolicy::Sandbox,
            shell_timeout_max: 300,
            ssrf_whitelist: vec![],
            mcp_servers: servers,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: DeviceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
        assert!(back.mcp_servers.contains_key("google"));
    }

    #[test]
    fn mcp_schemas_roundtrip() {
        let s = McpSchemas {
            server_name: "minimax".into(),
            tools: vec![ToolDef {
                name: "web_search".into(),
                input_schema: serde_json::json!({"type": "object"}),
                description: Some("Search the web".into()),
            }],
            resources: vec![ResourceDef {
                name: "page".into(),
                uri: "minimax://page/{page_id}".into(),
                description: None,
                mime_type: None,
            }],
            prompts: vec![PromptDef {
                name: "code_review".into(),
                arguments: vec![PromptArgument {
                    name: "language".into(),
                    description: None,
                    required: true,
                }],
                description: None,
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: McpSchemas = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn empty_mcp_schemas_serializes_with_empty_arrays() {
        let s = McpSchemas {
            server_name: "empty".into(),
            tools: vec![],
            resources: vec![],
            prompts: vec![],
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"tools\":[]"));
        assert!(json.contains("\"resources\":[]"));
        assert!(json.contains("\"prompts\":[]"));
    }
}
