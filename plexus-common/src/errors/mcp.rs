//! MCP-specific errors. See ADR-047, ADR-049, ADR-105.

use crate::errors::{Code, ErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("schema for '{wrapped_name}' differs across install sites")]
    SchemaCollision { wrapped_name: String },

    #[error("MCP server '{server}' advertises duplicate name: '{wrapped_name}'")]
    WithinServerCollision {
        server: String,
        wrapped_name: String,
    },

    #[error("MCP server '{server}' failed to spawn: {detail}")]
    SpawnFailed { server: String, detail: String },
}

impl Code for McpError {
    fn code(&self) -> ErrorCode {
        match self {
            McpError::SchemaCollision { .. } => ErrorCode::SchemaCollision,
            McpError::WithinServerCollision { .. } => ErrorCode::WithinServerCollision,
            McpError::SpawnFailed { .. } => ErrorCode::SpawnFailed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_collision_maps() {
        let e = McpError::SchemaCollision {
            wrapped_name: "mcp_google_search".into(),
        };
        assert_eq!(e.code(), ErrorCode::SchemaCollision);
    }

    #[test]
    fn within_server_collision_maps() {
        let e = McpError::WithinServerCollision {
            server: "google".into(),
            wrapped_name: "mcp_google_search".into(),
        };
        assert_eq!(e.code(), ErrorCode::WithinServerCollision);
    }

    #[test]
    fn spawn_failed_displays_server_and_detail() {
        let e = McpError::SpawnFailed {
            server: "google".into(),
            detail: "GOOGLE_API_KEY env var not set".into(),
        };
        let disp = format!("{}", e);
        assert!(disp.contains("google"));
        assert!(disp.contains("GOOGLE_API_KEY"));
    }
}
