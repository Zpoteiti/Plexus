//! Typed error enums + the wire-stable `ErrorCode`. See ADR-046.
//!
//! Every error type in this module implements the `Code` trait so any error
//! can be rendered to the wire via its stable `ErrorCode`.

use serde::{Deserialize, Serialize};

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

/// Stable wire-level error code. Serialized as `snake_case` strings.
///
/// New variants are additive; never repurpose an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    // Workspace
    NotFound,
    SoftLocked,
    UploadTooLarge,
    PathOutsideWorkspace,
    IoError,

    // Tool
    ExecTimeout,
    SandboxFailure,
    McpUnavailable,
    McpRestarting,
    CwdOutsideWorkspace,
    InvalidArgs,
    DeviceUnreachable,
    ClientShuttingDown,

    // Auth
    TokenInvalid,
    TokenExpired,
    Unauthorized,
    Forbidden,

    // Protocol
    MalformedFrame,
    UnknownType,
    VersionMismatch,
    TransferUnknownId,

    // MCP
    SchemaCollision,
    WithinServerCollision,
    SpawnFailed,

    // Network
    PrivateAddressBlocked,
    WhitelistMiss,
    DnsFailed,
    Timeout,
    HttpError,
}

/// Implemented by every typed error in this crate.
///
/// Maps the error variant to its wire-stable `ErrorCode`.
pub trait Code {
    fn code(&self) -> ErrorCode;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_serialize_matches_lowercase_snake() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::TokenInvalid).unwrap(),
            "\"token_invalid\""
        );
    }

    #[test]
    fn error_code_deserialize_from_lowercase_snake() {
        let parsed: ErrorCode = serde_json::from_str("\"path_outside_workspace\"").unwrap();
        assert_eq!(parsed, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn error_code_roundtrip_all_variants() {
        let variants = [
            ErrorCode::NotFound,
            ErrorCode::SoftLocked,
            ErrorCode::UploadTooLarge,
            ErrorCode::PathOutsideWorkspace,
            ErrorCode::IoError,
            ErrorCode::ExecTimeout,
            ErrorCode::SandboxFailure,
            ErrorCode::McpUnavailable,
            ErrorCode::McpRestarting,
            ErrorCode::CwdOutsideWorkspace,
            ErrorCode::InvalidArgs,
            ErrorCode::DeviceUnreachable,
            ErrorCode::ClientShuttingDown,
            ErrorCode::TokenInvalid,
            ErrorCode::TokenExpired,
            ErrorCode::Unauthorized,
            ErrorCode::Forbidden,
            ErrorCode::MalformedFrame,
            ErrorCode::UnknownType,
            ErrorCode::VersionMismatch,
            ErrorCode::TransferUnknownId,
            ErrorCode::SchemaCollision,
            ErrorCode::WithinServerCollision,
            ErrorCode::SpawnFailed,
            ErrorCode::PrivateAddressBlocked,
            ErrorCode::WhitelistMiss,
            ErrorCode::DnsFailed,
            ErrorCode::Timeout,
            ErrorCode::HttpError,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let back: ErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, back, "roundtrip failed for {:?}", variant);
        }
    }
}
