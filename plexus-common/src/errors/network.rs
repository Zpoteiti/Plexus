//! Network-layer errors. Raised by `web_fetch` and MCP transports.
//! See ADR-052.

use crate::errors::{Code, ErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("blocked: target IP {0} is in the private-address block-list")]
    PrivateAddressBlocked(String),

    #[error("blocked: target {0} is not in the device's ssrf_whitelist")]
    WhitelistMiss(String),

    #[error("DNS resolution failed for '{0}'")]
    DnsFailed(String),

    #[error("network operation timed out after {seconds}s")]
    Timeout { seconds: u32 },

    #[error("HTTP error: status {status}, body {body}")]
    HttpError { status: u16, body: String },
}

impl Code for NetworkError {
    fn code(&self) -> ErrorCode {
        match self {
            NetworkError::PrivateAddressBlocked(_) => ErrorCode::PrivateAddressBlocked,
            NetworkError::WhitelistMiss(_) => ErrorCode::WhitelistMiss,
            NetworkError::DnsFailed(_) => ErrorCode::DnsFailed,
            NetworkError::Timeout { .. } => ErrorCode::Timeout,
            NetworkError::HttpError { .. } => ErrorCode::HttpError,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_address_maps() {
        let e = NetworkError::PrivateAddressBlocked("10.0.0.1".into());
        assert_eq!(e.code(), ErrorCode::PrivateAddressBlocked);
    }

    #[test]
    fn http_error_displays_status_and_body() {
        let e = NetworkError::HttpError {
            status: 404,
            body: "not found".into(),
        };
        let disp = format!("{}", e);
        assert!(disp.contains("404"));
        assert!(disp.contains("not found"));
    }

    #[test]
    fn all_variants_map() {
        assert_eq!(
            NetworkError::WhitelistMiss("foo".into()).code(),
            ErrorCode::WhitelistMiss
        );
        assert_eq!(
            NetworkError::DnsFailed("foo".into()).code(),
            ErrorCode::DnsFailed
        );
        assert_eq!(
            NetworkError::Timeout { seconds: 30 }.code(),
            ErrorCode::Timeout
        );
        assert_eq!(
            NetworkError::HttpError {
                status: 500,
                body: "x".into()
            }
            .code(),
            ErrorCode::HttpError
        );
    }
}
