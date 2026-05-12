use crate::{
    auth::{AuthUser, password},
    db::users,
    error::ApiError,
};
use axum::{Json, extract::State};
use plexus_common::ErrorCode;
use serde::Deserialize;

pub async fn get_me(auth: AuthUser) -> Json<users::User> {
    Json(auth.user)
}

#[derive(Deserialize)]
pub struct PatchMeRequest {
    name: Option<String>,
    email: Option<String>,
    password: Option<String>,
}

pub async fn patch_me(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Json(req): Json<PatchMeRequest>,
) -> Result<Json<users::User>, ApiError> {
    let password_hash = match req.password.as_deref() {
        Some(password) if password.len() < 8 => {
            return Err(ApiError::invalid_args(
                "password must be at least 8 characters",
            ));
        }
        Some(password) => Some(
            password::hash_password(password)
                .map_err(|_| ApiError::invalid_args("password could not be hashed"))?,
        ),
        None => None,
    };
    let user = users::update_profile(
        state.pool(),
        auth.user.id,
        req.email.as_deref(),
        req.name.as_deref(),
        password_hash.as_deref(),
    )
    .await
    .map_err(map_update_user_error)?;
    Ok(Json(user))
}

fn map_update_user_error(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db_err) = &err {
        if db_err.is_unique_violation() {
            return ApiError::new(
                axum::http::StatusCode::CONFLICT,
                ErrorCode::InvalidArgs,
                "email already in use",
            );
        }
    }
    ApiError::from_sqlx(err)
}
