use super::validation;
use crate::{
    app::AppState,
    auth::{jwt, password},
    db::users,
    error::ApiError,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
};
use plexus_common::ErrorCode;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct RegisterRequest {
    email: String,
    password: String,
    name: String,
    admin_token: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    jwt: String,
    user: users::User,
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, HeaderMap, Json<AuthResponse>), ApiError> {
    validate_register(&req)?;
    let hash = password::hash_password(&req.password)
        .map_err(|_| ApiError::invalid_args("password could not be hashed"))?;
    let is_admin = state
        .config()
        .admin_token
        .as_ref()
        .is_some_and(|token| req.admin_token.as_deref() == Some(token.expose_secret()));
    let user = users::create_user(state.pool(), &req.email, &hash, &req.name, is_admin)
        .await
        .map_err(map_create_user_error)?;
    let dir = state.config().workspace_root.join(user.id.to_string());
    if let Err(err) = tokio::fs::create_dir_all(&dir).await {
        let _ = users::delete_by_id(state.pool(), user.id).await;
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::IoError,
            format!("workspace creation failed: {err}"),
        ));
    }
    let token = jwt::issue_token(
        state.config().jwt_secret.expose_secret(),
        user.id,
        user.is_admin,
    )
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::IoError,
            "token issue failed",
        )
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        jwt::session_cookie(&token, state.config().cookie_secure)
            .parse()
            .unwrap(),
    );
    Ok((
        StatusCode::CREATED,
        headers,
        Json(AuthResponse { jwt: token, user }),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<(HeaderMap, Json<AuthResponse>), ApiError> {
    let found = users::find_by_email(state.pool(), &req.email)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthorized,
                "invalid email or password",
            )
        })?;
    let ok = password::verify_password(&req.password, &found.password_hash).map_err(|_| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            "invalid email or password",
        )
    })?;
    if !ok {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            "invalid email or password",
        ));
    }
    let user = users::User {
        id: found.id,
        email: found.email,
        name: found.name,
        is_admin: found.is_admin,
        created_at: found.created_at,
    };
    let token = jwt::issue_token(
        state.config().jwt_secret.expose_secret(),
        user.id,
        user.is_admin,
    )
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::IoError,
            "token issue failed",
        )
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        jwt::session_cookie(&token, state.config().cookie_secure)
            .parse()
            .unwrap(),
    );
    Ok((headers, Json(AuthResponse { jwt: token, user })))
}

pub async fn logout(State(state): State<AppState>) -> (StatusCode, HeaderMap) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        jwt::clear_session_cookie(state.config().cookie_secure)
            .parse()
            .unwrap(),
    );
    (StatusCode::NO_CONTENT, headers)
}

fn validate_register(req: &RegisterRequest) -> Result<(), ApiError> {
    validation::email(&req.email)?;
    validation::password(&req.password)?;
    validation::name(&req.name)?;
    Ok(())
}

fn map_create_user_error(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db_err) = &err
        && db_err.is_unique_violation()
    {
        return ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::InvalidArgs,
            "email already in use",
        );
    }
    ApiError::from_sqlx(err)
}
