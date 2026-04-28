//! Errors raised during tool dispatch and execution. See ADR-031, ADR-046,
//! ADR-073, ADR-105.

use crate::errors::{Code, ErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool execution timed out after {seconds}s")]
    ExecTimeout { seconds: u32 },

    #[error("sandbox setup failed: {0}")]
    SandboxFailure(String),

    #[error("MCP server '{server}' is not running. Last error: {last_error}")]
    McpUnavailable {
        server: String,
        last_error: String,
    },

    #[error("MCP server '{server}' is restarting; try again in a moment")]
    McpRestarting { server: String },

    #[error("working directory {0} resolves outside the workspace")]
    CwdOutsideWorkspace(String),

    #[error("invalid args: {0}")]
    InvalidArgs(String),

    #[error("device '{device}' is unreachable")]
    DeviceUnreachable { device: String },

    #[error("client process is shutting down")]
    ClientShuttingDown,
}

impl Code for ToolError {
    fn code(&self) -> ErrorCode {
        match self {
            ToolError::ExecTimeout { .. } => ErrorCode::ExecTimeout,
            ToolError::SandboxFailure(_) => ErrorCode::SandboxFailure,
            ToolError::McpUnavailable { .. } => ErrorCode::McpUnavailable,
            ToolError::McpRestarting { .. } => ErrorCode::McpRestarting,
            ToolError::CwdOutsideWorkspace(_) => ErrorCode::CwdOutsideWorkspace,
            ToolError::InvalidArgs(_) => ErrorCode::InvalidArgs,
            ToolError::DeviceUnreachable { .. } => ErrorCode::DeviceUnreachable,
            ToolError::ClientShuttingDown => ErrorCode::ClientShuttingDown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_timeout_maps() {
        assert_eq!(
            ToolError::ExecTimeout { seconds: 60 }.code(),
            ErrorCode::ExecTimeout
        );
    }

    #[test]
    fn mcp_unavailable_maps_and_displays() {
        let e = ToolError::McpUnavailable {
            server: "google".into(),
            last_error: "GOOGLE_API_KEY env var not set".into(),
        };
        assert_eq!(e.code(), ErrorCode::McpUnavailable);
        assert!(format!("{}", e).contains("google"));
        assert!(format!("{}", e).contains("GOOGLE_API_KEY"));
    }

    #[test]
    fn device_unreachable_maps() {
        let e = ToolError::DeviceUnreachable {
            device: "mac-mini".into(),
        };
        assert_eq!(e.code(), ErrorCode::DeviceUnreachable);
    }

    #[test]
    fn client_shutting_down_maps() {
        assert_eq!(
            ToolError::ClientShuttingDown.code(),
            ErrorCode::ClientShuttingDown
        );
    }
}
