//! Typed error hierarchy shared by all Plexus crates.
//!
//! `ErrorCode` is the wire-level discriminant used in `ApiError` (HTTP) and
//! `ProtocolMessage::Error` (WebSocket). Each domain-specific typed error
//! (`WorkspaceError`, `ToolError`, ...) maps to one `ErrorCode` via `fn code()`.
//!
//! HTTP mapping (`ApiError → StatusCode`) lives in `plexus-server`; the server
//! layer wraps these typed errors into HTTP. Never define new error types
//! outside this tree.

use serde::{Deserialize, Serialize};
use std::fmt;

pub mod auth;
pub mod mcp;
pub mod network;
pub mod protocol;
pub mod tool;
pub mod workspace;

pub use auth::AuthError;
pub use mcp::McpError;
pub use network::NetworkError;
pub use protocol::ProtocolError;
pub use tool::ToolError;
pub use workspace::WorkspaceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    AuthFailed,
    AuthTokenExpired,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    ValidationFailed,
    InvalidParams,
    ExecutionFailed,
    DeviceOffline,
    ProtocolMismatch,
    InternalError,
    ToolTimeout,
    McpConnectionFailed,
    ConnectionFailed,
    HandshakeFailed,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthFailed => "AUTH_FAILED",
            Self::AuthTokenExpired => "AUTH_TOKEN_EXPIRED",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::NotFound => "NOT_FOUND",
            Self::Conflict => "CONFLICT",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::InvalidParams => "INVALID_PARAMS",
            Self::ExecutionFailed => "EXECUTION_FAILED",
            Self::DeviceOffline => "DEVICE_OFFLINE",
            Self::ProtocolMismatch => "PROTOCOL_MISMATCH",
            Self::InternalError => "INTERNAL_ERROR",
            Self::ToolTimeout => "TOOL_TIMEOUT",
            Self::McpConnectionFailed => "MCP_CONNECTION_FAILED",
            Self::ConnectionFailed => "CONNECTION_FAILED",
            Self::HandshakeFailed => "HANDSHAKE_FAILED",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::AuthFailed | Self::AuthTokenExpired | Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::ValidationFailed | Self::InvalidParams | Self::ProtocolMismatch => 400,
            Self::ToolTimeout => 504,
            Self::DeviceOffline => 503,
            Self::McpConnectionFailed | Self::ConnectionFailed | Self::HandshakeFailed => 502,
            Self::ExecutionFailed | Self::InternalError => 500,
        }
    }

    pub fn parse(s: &str) -> Option<ErrorCode> {
        match s {
            "AUTH_FAILED" => Some(Self::AuthFailed),
            "AUTH_TOKEN_EXPIRED" => Some(Self::AuthTokenExpired),
            "UNAUTHORIZED" => Some(Self::Unauthorized),
            "FORBIDDEN" => Some(Self::Forbidden),
            "NOT_FOUND" => Some(Self::NotFound),
            "CONFLICT" => Some(Self::Conflict),
            "VALIDATION_FAILED" => Some(Self::ValidationFailed),
            "INVALID_PARAMS" => Some(Self::InvalidParams),
            "EXECUTION_FAILED" => Some(Self::ExecutionFailed),
            "DEVICE_OFFLINE" => Some(Self::DeviceOffline),
            "PROTOCOL_MISMATCH" => Some(Self::ProtocolMismatch),
            "INTERNAL_ERROR" => Some(Self::InternalError),
            "TOOL_TIMEOUT" => Some(Self::ToolTimeout),
            "MCP_CONNECTION_FAILED" => Some(Self::McpConnectionFailed),
            "CONNECTION_FAILED" => Some(Self::ConnectionFailed),
            "HANDSHAKE_FAILED" => Some(Self::HandshakeFailed),
            _ => None,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code.as_str().to_string(),
            message: message.into(),
        }
    }

    pub fn http_status_code(&self) -> u16 {
        ErrorCode::parse(&self.code)
            .map(|c| c.http_status())
            .unwrap_or(500)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct PlexusError {
    pub code: ErrorCode,
    pub message: String,
}

impl PlexusError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for PlexusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for PlexusError {}

impl From<PlexusError> for ApiError {
    fn from(e: PlexusError) -> Self {
        ApiError::new(e.code, e.message)
    }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.http_status_code())
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_string(&self).unwrap_or_default();
        (status, [("content-type", "application/json")], body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_round_trip() {
        let code = ErrorCode::AuthFailed;
        assert_eq!(ErrorCode::parse(code.as_str()), Some(code));
    }

    #[test]
    fn test_all_codes_have_valid_http_status() {
        let codes = [
            ErrorCode::AuthFailed,
            ErrorCode::NotFound,
            ErrorCode::InternalError,
            ErrorCode::DeviceOffline,
            ErrorCode::ToolTimeout,
        ];
        for code in codes {
            let s = code.http_status();
            assert!((400..600).contains(&s), "Bad status for {code}: {s}");
        }
    }

    #[test]
    fn test_api_error_display() {
        let err = ApiError::new(ErrorCode::NotFound, "missing");
        assert_eq!(err.to_string(), "[NOT_FOUND] missing");
        assert_eq!(err.http_status_code(), 404);
    }

    #[test]
    fn test_plexus_error_into_api_error() {
        let ne = PlexusError::new(ErrorCode::InternalError, "oops");
        let ae: ApiError = ne.into();
        assert_eq!(ae.code, "INTERNAL_ERROR");
    }
}
