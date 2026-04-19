//! URL / IP validation errors for SSRF protection.
//!
//! Raised by `crate::network::validate_url` when a URL fails parsing,
//! uses a disallowed scheme, lacks a host, cannot be resolved, or resolves
//! to an IP in a blocked CIDR range. Conceptually distinct from
//! `ProtocolError` (which covers WebSocket frame/handshake issues).

use super::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// URL string failed to parse.
    #[error("invalid URL")]
    InvalidUrl,

    /// Scheme is not `http` or `https`.
    #[error("invalid URL scheme: only http/https allowed")]
    InvalidScheme,

    /// URL has no host component.
    #[error("URL missing host")]
    MissingHost,

    /// DNS / socket-address resolution failed.
    #[error("failed to resolve host")]
    ResolutionFailed,

    /// Host resolved to an IP in a blocked network (RFC-1918, link-local,
    /// loopback, metadata endpoint, IPv6 private, etc.) and no whitelist
    /// entry punched a hole for it.
    #[error("blocked network: {0}")]
    BlockedNetwork(std::net::IpAddr),
}

impl NetworkError {
    pub fn code(&self) -> ErrorCode {
        match self {
            NetworkError::InvalidUrl | NetworkError::InvalidScheme | NetworkError::MissingHost => {
                ErrorCode::InvalidParams
            }
            NetworkError::ResolutionFailed => ErrorCode::ValidationFailed,
            NetworkError::BlockedNetwork(_) => ErrorCode::Forbidden,
        }
    }
}
