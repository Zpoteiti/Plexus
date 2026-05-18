use crate::{app::AppState, auth::AuthUser, error::ApiError, workspace::QuotaState};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct WorkspaceDeviceQuery {
    plexus_device: Option<String>,
}

pub async fn quota(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<QuotaState>, ApiError> {
    Ok(Json(state.workspace_fs().quota(auth.user.id).await?))
}

pub async fn get_file(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
) -> Result<Response, ApiError> {
    require_server_device(&query)?;
    let bytes = state.workspace_fs().read_file(auth.user.id, &path).await?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

pub async fn put_file(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    require_server_device(&query)?;
    state
        .workspace_fs()
        .write_file(auth.user.id, &path, body.to_vec())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_file(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
) -> Result<StatusCode, ApiError> {
    require_server_device(&query)?;
    state
        .workspace_fs()
        .delete_file(auth.user.id, &path)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn require_server_device(query: &WorkspaceDeviceQuery) -> Result<(), ApiError> {
    match query.plexus_device.as_deref() {
        Some("server") => Ok(()),
        Some(_) => Err(ApiError::invalid_args(
            "M1d workspace REST only supports plexus_device=server",
        )),
        None => Err(ApiError::invalid_args(
            "plexus_device query parameter is required",
        )),
    }
}
