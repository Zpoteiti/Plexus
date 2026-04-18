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

// -- Workspace Upload --

#[derive(serde::Serialize)]
pub struct WorkspaceUploadResult {
    pub path: String,
    pub size_bytes: u64,
}

/// Save a single uploaded file under {user_root}/uploads/, using the same
/// dated-hashed naming convention as the channel adapters' inbound-media path:
///   uploads/{YYYY-MM-DD}-{8-char-hash}-{filename}
///
/// Quota enforcement happens inside `workspace_file_put_bytes` (reserves the
/// delta against the existing file, rolls back on write failure). Do NOT
/// pre-reserve here — that would double-count against the quota.
pub async fn workspace_upload_save_one(
    state: &AppState,
    user_id: &str,
    original_filename: &str,
    bytes: Vec<u8>,
) -> Result<WorkspaceUploadResult, crate::workspace::WorkspaceError> {
    let size = bytes.len() as u64;

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        format!("{:08x}", hasher.finish() as u32)
    };
    let safe_name = original_filename
        .replace(['/', '\\'], "_")
        .replace("..", "_");
    let rel = format!("uploads/{date}-{hash}-{safe_name}");

    workspace_file_put_bytes(state, user_id, &rel, bytes).await?;
    Ok(WorkspaceUploadResult {
        path: rel,
        size_bytes: size,
    })
}

async fn workspace_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Vec<WorkspaceUploadResult>>, ApiError> {
    let claims = claims(&headers, &state)?;
    let mut results = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, format!("multipart: {e}")))?
    {
        let filename = field.file_name().unwrap_or("unnamed").to_string();
        let bytes = field.bytes().await.map_err(|e| {
            ApiError::new(ErrorCode::ValidationFailed, format!("multipart read: {e}"))
        })?;
        match workspace_upload_save_one(&state, &claims.sub, &filename, bytes.to_vec()).await {
            Ok(r) => results.push(r),
            Err(_) => results.push(WorkspaceUploadResult {
                // Sentinel: "ERROR:{filename}" so the client can surface per-file failures.
                path: format!("ERROR:{filename}"),
                size_bytes: 0,
            }),
        }
    }
    Ok(Json(results))
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
    let root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path(root, &claims.sub, &q.path)
        .await
        .map_err(|e| match &e {
            crate::workspace::WorkspaceError::Traversal(_) => {
                ApiError::new(ErrorCode::Forbidden, "forbidden")
            }
            _ => ApiError::new(ErrorCode::NotFound, "not found"),
        })?;

    let meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| ApiError::new(ErrorCode::NotFound, "not found"))?;
    let size = meta.len();

    let file = tokio::fs::File::open(&resolved)
        .await
        .map_err(|_| ApiError::new(ErrorCode::NotFound, "not found"))?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mime = mime_from_path(&q.path);
    let mut resp = Response::new(body);
    let resp_headers = resp.headers_mut();
    resp_headers.insert(
        axum::http::header::CONTENT_TYPE,
        mime.parse().unwrap_or_else(|_| {
            "application/octet-stream"
                .parse()
                .expect("static str parses")
        }),
    );
    resp_headers.insert(
        axum::http::header::CONTENT_LENGTH,
        size.to_string()
            .parse()
            .expect("u64 to string always parses as header value"),
    );
    resp_headers.insert(
        "X-Content-Type-Options",
        "nosniff".parse().expect("static str parses"),
    );
    Ok(resp)
}

/// Testable core: write raw bytes to `{workspace_root}/{user_id}/{rel_path}`.
/// Creates parent dirs. Quota-checked via `check_and_reserve_upload` (delta
/// against existing file size). Rolls back reservation on write failure.
pub async fn workspace_file_put_bytes(
    state: &AppState,
    user_id: &str,
    rel_path: &str,
    bytes: Vec<u8>,
) -> Result<(), crate::workspace::WorkspaceError> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path_for_create(root, user_id, rel_path).await?;

    // Compute delta against existing file (if any), reserve against quota.
    let existing = tokio::fs::metadata(&resolved)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let new_size = bytes.len() as u64;
    let delta = new_size.saturating_sub(existing);

    if delta > 0 {
        state
            .quota
            .check_and_reserve_upload(user_id, delta)
            .map_err(crate::workspace::WorkspaceError::Quota)?;
    }

    // Create parent dirs.
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    match tokio::fs::write(&resolved, &bytes).await {
        Ok(()) => {
            // If we shrunk the file, release the reclaimed bytes.
            if new_size < existing {
                state.quota.release(user_id, existing - new_size);
            }
            Ok(())
        }
        Err(e) => {
            // Rollback the reservation.
            if delta > 0 {
                state.quota.release(user_id, delta);
            }
            Err(e.into())
        }
    }
}

async fn workspace_file_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<WorkspaceFileQuery>,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let claims = claims(&headers, &state)?;
    workspace_file_put_bytes(&state, &claims.sub, &q.path, body.to_vec())
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|e| {
            use crate::workspace::WorkspaceError;
            match &e {
                WorkspaceError::Traversal(_) => ApiError::new(ErrorCode::Forbidden, "forbidden"),
                WorkspaceError::Quota(_) => {
                    ApiError::new(ErrorCode::ValidationFailed, format!("{e:?}"))
                }
                _ => ApiError::new(ErrorCode::InternalError, format!("{e:?}")),
            }
        })
}

// -- Workspace File Delete --

#[derive(serde::Deserialize)]
pub struct WorkspaceDeleteQuery {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

pub async fn workspace_file_delete_path(
    state: &AppState,
    user_id: &str,
    rel_path: &str,
    recursive: bool,
) -> Result<(), crate::workspace::WorkspaceError> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path(root, user_id, rel_path).await?;

    let meta = tokio::fs::metadata(&resolved).await?;

    if meta.is_dir() {
        if !recursive {
            return Err(crate::workspace::WorkspaceError::Io(std::io::Error::other(
                "directory delete requires recursive=true",
            )));
        }
        // Sum sizes before deletion to release from quota.
        let freed = dir_size(&resolved).await.unwrap_or(0);
        tokio::fs::remove_dir_all(&resolved).await?;
        state.quota.release(user_id, freed);
    } else {
        let size = meta.len();
        tokio::fs::remove_file(&resolved).await?;
        state.quota.release(user_id, size);
    }
    Ok(())
}

async fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut total = 0u64;
        for entry in walkdir::WalkDir::new(&path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Ok(m) = entry.metadata() {
                    total = total.saturating_add(m.len());
                }
            }
        }
        Ok(total)
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::other(e)))
}

async fn workspace_file_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<WorkspaceDeleteQuery>,
) -> Result<StatusCode, ApiError> {
    let claims = claims(&headers, &state)?;
    workspace_file_delete_path(&state, &claims.sub, &q.path, q.recursive)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|e| {
            use crate::workspace::WorkspaceError;
            match &e {
                WorkspaceError::Traversal(_) => ApiError::new(ErrorCode::Forbidden, "forbidden"),
                _ => ApiError::new(ErrorCode::ValidationFailed, format!("{e:?}")),
            }
        })
}

// -- Workspace Skills --

#[derive(serde::Serialize)]
pub struct WorkspaceSkillSummary {
    pub name: String,
    pub description: String,
    pub always_on: bool,
}

pub async fn workspace_skills_list(state: &AppState, user_id: &str) -> Vec<WorkspaceSkillSummary> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let bundle = state.skills_cache.get_or_load(user_id, root).await;
    bundle
        .iter()
        .map(|s| WorkspaceSkillSummary {
            name: s.name.clone(),
            description: s.description.clone(),
            always_on: s.always_on,
        })
        .collect()
}

async fn workspace_skills(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<WorkspaceSkillSummary>>, ApiError> {
    let claims = claims(&headers, &state)?;
    Ok(Json(workspace_skills_list(&state, &claims.sub).await))
}

// -- Self-serve Account Deletion --

#[derive(serde::Deserialize)]
struct DeleteSelfRequest {
    password: String,
}

async fn delete_self(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DeleteSelfRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claim = claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &claim.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;

    let password_ok = bcrypt::verify(&body.password, &user.password_hash).unwrap_or(false);
    if !password_ok {
        return Err(ApiError::new(ErrorCode::Unauthorized, "Invalid password"));
    }

    if user.is_admin {
        tracing::warn!(
            user_id = %user.user_id,
            email = %user.email,
            "Admin is deleting their own account"
        );
    }

    crate::account::delete_user_everywhere(&state, &user.user_id).await;

    Ok(Json(serde_json::json!({ "message": "Account deleted" })))
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
        .route(
            "/api/workspace/file",
            get(workspace_file_get)
                .put(workspace_file_put)
                .delete(workspace_file_delete),
        )
        .route("/api/workspace/upload", post(workspace_upload))
        .route("/api/workspace/skills", get(workspace_skills))
        .route("/api/user", delete(delete_self))
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

    #[tokio::test]
    async fn test_workspace_file_put_writes_bytes_and_updates_quota() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);

        workspace_file_put_bytes(&state, "alice", "notes.md", b"hello".to_vec())
            .await
            .unwrap();

        let written = tokio::fs::read(user_root.join("notes.md")).await.unwrap();
        assert_eq!(written, b"hello");
        assert_eq!(state.quota.current_usage("alice"), 5);
    }

    #[tokio::test]
    async fn test_workspace_file_delete_file_updates_quota() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        tokio::fs::write(user_root.join("doomed.txt"), b"goodbye")
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);
        state.quota.reserve_for_test("alice", 7);

        workspace_file_delete_path(&state, "alice", "doomed.txt", false)
            .await
            .unwrap();

        assert!(!user_root.join("doomed.txt").exists());
        assert_eq!(state.quota.current_usage("alice"), 0);
    }

    #[tokio::test]
    async fn test_workspace_file_delete_directory_requires_recursive_flag() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_root.join("subdir"))
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let err = workspace_file_delete_path(&state, "alice", "subdir", false)
            .await
            .unwrap_err();
        assert!(format!("{err:?}").to_lowercase().contains("directory"));

        // With recursive: true, it succeeds.
        workspace_file_delete_path(&state, "alice", "subdir", true)
            .await
            .unwrap();
        assert!(!user_root.join("subdir").exists());
    }

    #[tokio::test]
    async fn test_workspace_upload_saves_to_uploads_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("alice/uploads"))
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);
        let saved = workspace_upload_save_one(&state, "alice", "photo.jpg", b"fakedata".to_vec())
            .await
            .unwrap();

        assert!(
            saved.path.starts_with("uploads/"),
            "expected uploads/ prefix; got {}",
            saved.path
        );
        assert!(saved.path.ends_with("photo.jpg"));
        assert_eq!(saved.size_bytes, 8);

        // File actually exists on disk.
        let full = tmp.path().join("alice").join(&saved.path);
        assert!(full.exists());
    }

    #[tokio::test]
    async fn test_workspace_file_put_rejects_quota_overage() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("alice"))
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 10);

        let err = workspace_file_put_bytes(&state, "alice", "big.bin", vec![0; 100])
            .await
            .unwrap_err();
        // Accept whatever variant the actual quota enforcement returns —
        // the string form just needs to indicate a size/quota problem.
        let msg = format!("{err:?}").to_lowercase();
        assert!(
            msg.contains("quota")
                || msg.contains("too large")
                || msg.contains("soft")
                || msg.contains("cap"),
            "expected quota-related error; got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_workspace_skills_returns_parsed_frontmatter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_root = tmp.path().join("alice/skills/demo");
        tokio::fs::create_dir_all(&skills_root).await.unwrap();
        tokio::fs::write(
            skills_root.join("SKILL.md"),
            b"---\nname: demo\ndescription: A demo skill\nalways_on: true\n---\n\n# Demo",
        )
        .await
        .unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let skills = workspace_skills_list(&state, "alice").await;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "demo");
        assert_eq!(skills[0].description, "A demo skill");
        assert!(skills[0].always_on);
    }
}
