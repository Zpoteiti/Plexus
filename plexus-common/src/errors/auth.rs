//! Authentication errors. Used at REST and WS handshake boundaries.

use crate::errors::{Code, ErrorCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("token is invalid or revoked")]
    TokenInvalid,

    #[error("token has expired")]
    TokenExpired,

    #[error("authentication required")]
    Unauthorized,

    #[error("authenticated but lacks permission")]
    Forbidden,
}

impl Code for AuthError {
    fn code(&self) -> ErrorCode {
        match self {
            AuthError::TokenInvalid => ErrorCode::TokenInvalid,
            AuthError::TokenExpired => ErrorCode::TokenExpired,
            AuthError::Unauthorized => ErrorCode::Unauthorized,
            AuthError::Forbidden => ErrorCode::Forbidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_map() {
        assert_eq!(AuthError::TokenInvalid.code(), ErrorCode::TokenInvalid);
        assert_eq!(AuthError::TokenExpired.code(), ErrorCode::TokenExpired);
        assert_eq!(AuthError::Unauthorized.code(), ErrorCode::Unauthorized);
        assert_eq!(AuthError::Forbidden.code(), ErrorCode::Forbidden);
    }
}
