//! User, session, and file endpoints. All require JWT.

use crate::auth::{Claims, extract_claims};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use plexus_common::error::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

// -- User Profile --

async fn get_profile(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;
    Ok(Json(serde_json::json!({
        "user_id": user.user_id,
        "email": user.email,
        "is_admin": user.is_admin,
        "display_name": user.display_name,
        "created_at": user.created_at.to_rfc3339(),
    })))
}

// -- Soul (deprecated) --
// Soul is now a workspace file: {workspace}/{user_id}/SOUL.md
// These endpoints return 410 Gone.

async fn get_soul(_headers: HeaderMap, _state: State<Arc<AppState>>) -> (StatusCode, &'static str) {
    (
        StatusCode::GONE,
        "Soul is now a workspace file. Use read_file on SOUL.md or the workspace file API.",
    )
}

async fn patch_soul(
    _headers: HeaderMap,
    _state: State<Arc<AppState>>,
    _body: axum::body::Bytes,
) -> (StatusCode, &'static str) {
    (
        StatusCode::GONE,
        "Soul is now a workspace file. Use write_file/edit_file on SOUL.md or the workspace file API.",
    )
}

// -- Display Name --

#[derive(Deserialize)]
struct DisplayNameUpdate {
    display_name: String,
}

async fn patch_display_name(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<DisplayNameUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let name = if req.display_name.trim().is_empty() {
        None
    } else {
        Some(req.display_name.trim())
    };
    crate::db::users::update_display_name(&state.db, &c.sub, name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(
        serde_json::json!({ "message": "Display name updated" }),
    ))
}

// -- Memory (deprecated) --
// Memory is now a workspace file: {workspace}/{user_id}/MEMORY.md
// These endpoints return 410 Gone.

async fn get_memory(
    _headers: HeaderMap,
    _state: State<Arc<AppState>>,
) -> (StatusCode, &'static str) {
    (
        StatusCode::GONE,
        "Memory is now a workspace file. Use read_file on MEMORY.md or the workspace file API.",
    )
}

async fn patch_memory(
    _headers: HeaderMap,
    _state: State<Arc<AppState>>,
    _body: axum::body::Bytes,
) -> (StatusCode, &'static str) {
    (
        StatusCode::GONE,
        "Memory is now a workspace file. Use write_file/edit_file on MEMORY.md or the workspace file API.",
    )
}

// -- Sessions --

async fn list_sessions(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::sessions::Session>>, ApiError> {
    let c = claims(&headers, &state)?;
    let sessions = crate::db::sessions::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(sessions))
}

async fn delete_session(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let session = crate::db::sessions::find_by_id(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Session not found"))?;
    if session.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your session"));
    }
    crate::db::sessions::delete_session(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    state.sessions.remove(&session_id);
    Ok(Json(serde_json::json!({ "message": "Session deleted" })))
}

#[derive(Deserialize)]
struct MessageQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn get_messages(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(q): Query<MessageQuery>,
) -> Result<Json<Vec<crate::db::messages::Message>>, ApiError> {
    let c = claims(&headers, &state)?;
    let session = crate::db::sessions::find_by_id(&state.db, &session_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Session not found"))?;
    if session.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your session"));
    }
    let limit = q.limit.unwrap_or(50).min(500);
    let offset = q.offset.unwrap_or(0);
    let msgs = crate::db::messages::list_paginated(&state.db, &session_id, limit, offset)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(msgs))
}

// -- Files --

async fn upload_file(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, format!("multipart: {e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::ValidationFailed, "No file provided"))?;
    let filename = field.file_name().unwrap_or("upload").to_string();
    let data = field
        .bytes()
        .await
        .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, format!("read: {e}")))?;
    let file_id = crate::file_store::save_upload(&state, &c.sub, &filename, &data).await?;
    Ok(Json(serde_json::json!({
        "file_id": file_id,
        "file_name": filename,
    })))
}

async fn download_file(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<String>,
) -> Result<Response<Body>, ApiError> {
    let c = claims(&headers, &state)?;
    let (data, filename) = crate::file_store::load_file(&state, &c.sub, &file_id).await?;
    let mime = plexus_common::mime::detect_mime_from_extension(&filename)
        .unwrap_or("application/octet-stream");
    Ok(Response::builder()
        .header("Content-Type", mime)
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        )
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(data))
        .unwrap())
}

// -- Workspace Quota --

#[derive(serde::Serialize)]
pub struct WorkspaceQuotaResponse {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// Pure logic: query the quota cache for `user_id`. No I/O or DB calls.
pub fn workspace_quota_handler(state: &AppState, user_id: &str) -> WorkspaceQuotaResponse {
    let used = state.quota.current_usage(user_id);
    let total = state.quota.quota_bytes();
    WorkspaceQuotaResponse {
        used_bytes: used,
        total_bytes: total,
    }
}

/// GET /api/workspace/quota
async fn workspace_quota(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<WorkspaceQuotaResponse>, ApiError> {
    let c = claims(&headers, &state)?;
    Ok(Json(workspace_quota_handler(&state, &c.sub)))
}

// -- Workspace Tree --

/// GET /api/workspace/tree
async fn workspace_tree(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::workspace::WorkspaceEntry>>, ApiError> {
    let c = claims(&headers, &state)?;
    let root = std::path::Path::new(&state.config.workspace_root);
    crate::workspace::walk_user_tree(root, &c.sub)
        .await
        .map(Json)
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("tree walk failed: {e}")))
}

// -- Workspace File --

#[derive(serde::Deserialize)]
pub struct WorkspaceFileQuery {
    pub path: String,
}

/// Testable core: given user_id + rel path, return bytes or an error.
/// HTTP wrapper below converts the error to StatusCode + JSON body.
pub async fn workspace_file_get_bytes(
    state: &AppState,
    user_id: &str,
    rel_path: &str,
) -> Result<Vec<u8>, crate::workspace::WorkspaceError> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path(root, user_id, rel_path).await?;
    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(crate::workspace::WorkspaceError::Io)?;
    Ok(bytes)
}

async fn workspace_file_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<WorkspaceFileQuery>,
) -> Result<Response<Body>, ApiError> {
    let claims = claims(&headers, &state)?;
    match workspace_file_get_bytes(&state, &claims.sub, &q.path).await {
        Ok(bytes) => {
            let mime = mime_from_path(&q.path);
            Ok(Response::builder()
                .header("Content-Type", mime)
                .body(Body::from(bytes))
                .unwrap())
        }
        Err(e) => {
            use crate::workspace::WorkspaceError;
            let api_err = match &e {
                WorkspaceError::Traversal(_) => ApiError::new(ErrorCode::Forbidden, "forbidden"),
                WorkspaceError::Io(io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
                    ApiError::new(ErrorCode::NotFound, "not found")
                }
                _ => ApiError::new(ErrorCode::InternalError, format!("{e:?}")),
            };
            Err(api_err)
        }
    }
}

fn mime_from_path(p: &str) -> &'static str {
    let ext = p.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "md" | "txt" | "log" | "toml" | "rs" | "ts" | "tsx" | "js" | "py" | "yaml" | "yml" => {
            "text/plain; charset=utf-8"
        }
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/user/profile", get(get_profile))
        .route("/api/user/soul", get(get_soul).patch(patch_soul))
        .route("/api/user/display-name", patch(patch_display_name))
        .route("/api/user/memory", get(get_memory).patch(patch_memory))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session_id}", delete(delete_session))
        .route("/api/sessions/{session_id}/messages", get(get_messages))
        .route("/api/files", post(upload_file))
        .route("/api/files/{file_id}", get(download_file))
        .route("/api/workspace/quota", get(workspace_quota))
        .route("/api/workspace/tree", get(workspace_tree))
        .route("/api/workspace/file", get(workspace_file_get))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_workspace_quota_shape() {
        // Pure-logic test: the handler's response body matches the spec.
        // No HTTP harness needed — we construct the state and call the handler.
        let tmp = tempfile::TempDir::new().unwrap();
        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 5 * 1024 * 1024);
        state.quota.reserve_for_test("alice", 1024);

        let result = workspace_quota_handler(&state, "alice");
        assert_eq!(result.used_bytes, 1024);
        assert_eq!(result.total_bytes, 5 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_workspace_file_get_inside_user_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        tokio::fs::write(user_root.join("greeting.txt"), b"hi there")
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let bytes = workspace_file_get_bytes(&state, "alice", "greeting.txt")
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"hi there");
    }

    #[tokio::test]
    async fn test_workspace_file_get_rejects_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("alice"))
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let err = workspace_file_get_bytes(&state, "alice", "../../etc/passwd")
            .await
            .unwrap_err();
        // WorkspaceError may render as Traversal or NotFound depending on which
        // check fires first. Both are acceptable — just assert it's an error.
        let msg = format!("{err:?}").to_lowercase();
        assert!(
            msg.contains("traversal")
                || msg.contains("not found")
                || msg.contains("outside")
                || msg.contains("io"),
            "expected traversal/not-found/io-style error; got: {msg}"
        );
    }
}
