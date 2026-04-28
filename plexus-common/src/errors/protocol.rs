//! WebSocket protocol errors.
//!
//! Raised when a peer sends a frame the receiver cannot interpret, or when
//! the handshake reveals an incompatible protocol version.

use super::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Incoming frame failed to deserialize or violated schema expectations.
    #[error("malformed frame: {0}")]
    MalformedFrame(String),

    /// Peer's protocol version is incompatible with ours.
    #[error("protocol version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: String, actual: String },
}

impl ProtocolError {
    pub fn code(&self) -> ErrorCode {
        match self {
            ProtocolError::MalformedFrame(_) => ErrorCode::ValidationFailed,
            ProtocolError::VersionMismatch { .. } => ErrorCode::ProtocolMismatch,
        }
    }
}
