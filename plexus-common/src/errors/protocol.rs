//! Wire-protocol errors raised by the WS frame layer (PROTOCOL.md §5.1).

use crate::errors::{Code, ErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("malformed frame: {0}")]
    MalformedFrame(String),

    #[error("unknown frame type: {0}")]
    UnknownType(String),

    #[error("protocol version mismatch: server requires {required}, client sent {client_sent}")]
    VersionMismatch {
        required: String,
        client_sent: String,
    },

    #[error("transfer slot {0} is not active")]
    TransferUnknownId(String),
}

impl Code for ProtocolError {
    fn code(&self) -> ErrorCode {
        match self {
            ProtocolError::MalformedFrame(_) => ErrorCode::MalformedFrame,
            ProtocolError::UnknownType(_) => ErrorCode::UnknownType,
            ProtocolError::VersionMismatch { .. } => ErrorCode::VersionMismatch,
            ProtocolError::TransferUnknownId(_) => ErrorCode::TransferUnknownId,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_mismatch_displays_both_versions() {
        let e = ProtocolError::VersionMismatch {
            required: "2".into(),
            client_sent: "1".into(),
        };
        let disp = format!("{}", e);
        assert!(disp.contains("2"));
        assert!(disp.contains("1"));
    }

    #[test]
    fn all_variants_map() {
        assert_eq!(
            ProtocolError::MalformedFrame("oops".into()).code(),
            ErrorCode::MalformedFrame
        );
        assert_eq!(
            ProtocolError::UnknownType("zzz".into()).code(),
            ErrorCode::UnknownType
        );
        assert_eq!(
            ProtocolError::VersionMismatch {
                required: "2".into(),
                client_sent: "1".into(),
            }
            .code(),
            ErrorCode::VersionMismatch
        );
        assert_eq!(
            ProtocolError::TransferUnknownId("uuid".into()).code(),
            ErrorCode::TransferUnknownId
        );
    }
}
