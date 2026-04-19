//! Authentication and authorization errors.
//!
//! Covers JWT / device-token validation and authorization checks. Additional
//! variants may land as later cleanup tasks unify the auth surface.

use super::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Token is structurally invalid or its signature did not verify.
    #[error("token invalid")]
    TokenInvalid,

    /// Token is well-formed but past its expiry.
    #[error("token expired")]
    TokenExpired,

    /// Caller is authenticated but not permitted to perform the action.
    #[error("forbidden")]
    Forbidden,
}

impl AuthError {
    pub fn code(&self) -> ErrorCode {
        match self {
            AuthError::TokenInvalid => ErrorCode::AuthFailed,
            AuthError::TokenExpired => ErrorCode::AuthTokenExpired,
            AuthError::Forbidden => ErrorCode::Forbidden,
        }
    }
}
