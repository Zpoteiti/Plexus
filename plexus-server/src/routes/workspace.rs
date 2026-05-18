use crate::{
    app::AppState,
    auth::AuthUser,
    error::ApiError,
    workspace::{DirEntry, QuotaState},
};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use plexus_common::WorkspaceError;
use serde::{Deserialize, Serialize};

pub const WORKSPACE_REST_UPLOAD_MEMORY_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
pub struct WorkspaceDeviceQuery {
    plexus_device: Option<String>,
}

#[derive(Deserialize)]
pub struct EditRequest {
    old_text: String,
    new_text: String,
    #[serde(default)]
    replace_all: bool,
}

#[derive(Serialize)]
pub struct EditResponse {
    replacements: usize,
}

#[derive(Deserialize)]
pub struct GlobQuery {
    plexus_device: Option<String>,
    pattern: String,
}

#[derive(Deserialize)]
pub struct GrepQuery {
    plexus_device: Option<String>,
    pattern: String,
    path: Option<String>,
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
    require_server_device(query.plexus_device.as_deref())?;
    let bytes = state.workspace_fs().read_file(auth.user.id, &path).await?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

pub async fn put_file(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
    body: Body,
) -> Result<StatusCode, ApiError> {
    require_server_device(query.plexus_device.as_deref())?;
    let quota = state.workspace_fs().quota(auth.user.id).await?;
    if quota.locked {
        return Err(WorkspaceError::SoftLocked.into());
    }
    let single_op_limit = quota.quota_bytes.saturating_mul(80) / 100;
    let collection_limit = single_op_limit.min(WORKSPACE_REST_UPLOAD_MEMORY_LIMIT_BYTES);
    let body = to_bytes(body, collection_limit as usize)
        .await
        .map_err(|_| WorkspaceError::UploadTooLarge {
            actual_bytes: collection_limit.saturating_add(1),
            quota_bytes: quota.quota_bytes,
        })?;

    state
        .workspace_fs()
        .write_file(auth.user.id, &path, body.to_vec())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn patch_file(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
    Json(request): Json<EditRequest>,
) -> Result<Json<EditResponse>, ApiError> {
    require_server_device(query.plexus_device.as_deref())?;
    let replacements = state
        .workspace_fs()
        .edit_file(
            auth.user.id,
            &path,
            &request.old_text,
            &request.new_text,
            request.replace_all,
        )
        .await?;
    Ok(Json(EditResponse { replacements }))
}

pub async fn delete_file(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
) -> Result<StatusCode, ApiError> {
    require_server_device(query.plexus_device.as_deref())?;
    state
        .workspace_fs()
        .delete_file(auth.user.id, &path)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_folder(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
) -> Result<StatusCode, ApiError> {
    require_server_device(query.plexus_device.as_deref())?;
    state
        .workspace_fs()
        .delete_folder(auth.user.id, &path)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_dir(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<WorkspaceDeviceQuery>,
) -> Result<Json<Vec<DirEntry>>, ApiError> {
    require_server_device(query.plexus_device.as_deref())?;
    Ok(Json(
        state.workspace_fs().list_dir(auth.user.id, &path).await?,
    ))
}

pub async fn glob(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<GlobQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    require_server_device(query.plexus_device.as_deref())?;
    Ok(Json(
        state
            .workspace_fs()
            .glob(auth.user.id, &query.pattern)
            .await?,
    ))
}

pub async fn grep(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<GrepQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    require_server_device(query.plexus_device.as_deref())?;
    Ok(Json(
        state
            .workspace_fs()
            .grep(auth.user.id, &query.pattern, query.path.as_deref())
            .await?,
    ))
}

fn require_server_device(device: Option<&str>) -> Result<(), ApiError> {
    match device {
        Some("server") => Ok(()),
        Some(_) => Err(ApiError::invalid_args(
            "M1d workspace REST only supports plexus_device=server",
        )),
        None => Err(ApiError::invalid_args(
            "plexus_device query parameter is required",
        )),
    }
}
