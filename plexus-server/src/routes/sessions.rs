use crate::{auth::AuthUser, db::sessions, error::ApiError};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use plexus_common::ErrorCode;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct SessionListQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    title: Option<String>,
}

#[derive(Deserialize)]
pub struct RenameSessionRequest {
    title: String,
}

pub async fn list_sessions(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<Vec<sessions::Session>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let rows = sessions::list_for_user(state.pool(), auth.user.id, limit, offset)
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(Json(rows))
}

pub async fn create_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<sessions::Session>), ApiError> {
    let title =
        sessions::normalize_create_title(req.title.as_deref()).map_err(ApiError::invalid_args)?;
    let session = sessions::create_web_session(state.pool(), auth.user.id, &title)
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok((StatusCode::CREATED, Json(session)))
}

pub async fn get_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<sessions::Session>, ApiError> {
    let session = owned_session_or_404(state.pool(), auth.user.id, session_id).await?;
    Ok(Json(session))
}

pub async fn rename_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<RenameSessionRequest>,
) -> Result<Json<sessions::Session>, ApiError> {
    let title = sessions::normalize_rename_title(&req.title).map_err(ApiError::invalid_args)?;
    let session = sessions::rename_owned(state.pool(), auth.user.id, session_id, &title)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(not_found)?;
    Ok(Json(session))
}

pub async fn delete_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let deleted = sessions::delete_owned(state.pool(), auth.user.id, session_id)
        .await
        .map_err(ApiError::from_sqlx)?;
    if !deleted {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn owned_session_or_404(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<sessions::Session, ApiError> {
    sessions::find_owned(pool, user_id, session_id)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(not_found)
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
        "session not found",
    )
}
