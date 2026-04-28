//! MCP (Model Context Protocol) integration errors.
//!
//! Raised when a configured MCP server is unreachable or when its advertised
//! tool schema collides with a built-in or another MCP server's tool.

use super::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// Could not connect to (or lost connection with) the named MCP server.
    #[error("MCP server unreachable: {0}")]
    ServerUnreachable(String),

    /// Two sources advertised the same tool name. `server` is the source
    /// being rejected; `tool` is the offending tool name.
    #[error("MCP tool name collision: {server} advertises tool `{tool}`")]
    SchemaCollision { server: String, tool: String },
}

impl McpError {
    pub fn code(&self) -> ErrorCode {
        match self {
            McpError::ServerUnreachable(_) => ErrorCode::McpConnectionFailed,
            McpError::SchemaCollision { .. } => ErrorCode::Conflict,
        }
    }
}
