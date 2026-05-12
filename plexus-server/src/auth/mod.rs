pub mod jwt;
pub mod password;

use crate::{app::AppState, db::users, error::ApiError};
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header, request::Parts},
};
use cookie::Cookie;
use plexus_common::ErrorCode;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user: users::User,
}

#[derive(Debug, Clone)]
pub struct AdminUser {
    pub user: users::User,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)
            .or_else(|| cookie_token(parts))
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::Unauthorized,
                    "authentication required",
                )
            })?;
        let claims = jwt::verify_token(&state.config().jwt_secret, &token).map_err(|_| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::TokenInvalid,
                "token is invalid or expired",
            )
        })?;
        let user = users::find_by_id(state.pool(), claims.sub)
            .await
            .map_err(ApiError::from_sqlx)?
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::TokenInvalid,
                    "token user no longer exists",
                )
            })?;
        Ok(Self { user })
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth = AuthUser::from_request_parts(parts, state).await?;
        if !auth.user.is_admin {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                ErrorCode::Forbidden,
                "authenticated but lacks permission",
            ));
        }
        Ok(Self { user: auth.user })
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(ToOwned::to_owned)
}

fn cookie_token(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(header::COOKIE)?.to_str().ok()?;
    for raw in header.split(';') {
        let cookie = Cookie::parse(raw.trim().to_string()).ok()?;
        if cookie.name() == "plexus_session" {
            return Some(cookie.value().to_string());
        }
    }
    None
}
