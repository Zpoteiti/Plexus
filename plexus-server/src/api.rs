//! User, session, and file endpoints. All require JWT.

use crate::auth::{Claims, extract_claims};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use plexus_common::errors::{ApiError, ErrorCode};
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
    let mime = plexus_common::mime::detect_mime_from_extension(&filename);
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
    pub filename: String,
    pub outcome: UploadOutcome,
}

#[derive(serde::Serialize, Debug)]
pub struct Uploaded {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UploadError {
    Quota { remaining: u64 },
    TooLarge,
    Io(String),
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UploadOutcome {
    Success(Uploaded),
    Error(UploadError),
}

/// Save a single uploaded file under {user_root}/uploads/, using the same
/// dated-hashed naming convention as the channel adapters' inbound-media path:
///   uploads/{YYYY-MM-DD}-{8-char-hash}-{filename}
pub async fn workspace_upload_save_one(
    state: &AppState,
    user_id: &str,
    original_filename: &str,
    bytes: Vec<u8>,
) -> WorkspaceUploadResult {
    use plexus_common::errors::workspace::WorkspaceError;

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

    let outcome = match state.workspace_fs.write(user_id, &rel, &bytes).await {
        Ok(()) => UploadOutcome::Success(Uploaded { path: rel, size_bytes: size }),
        Err(WorkspaceError::UploadTooLarge { .. }) => UploadOutcome::Error(UploadError::TooLarge),
        Err(WorkspaceError::SoftLocked) => {
            let snap = state.workspace_fs.quota(user_id);
            let remaining = snap.limit_bytes.saturating_sub(snap.used_bytes);
            UploadOutcome::Error(UploadError::Quota { remaining })
        }
        Err(e) => UploadOutcome::Error(UploadError::Io(format!("{e}"))),
    };
    WorkspaceUploadResult { filename: original_filename.to_string(), outcome }
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
        results.push(workspace_upload_save_one(&state, &claims.sub, &filename, bytes.to_vec()).await);
    }
    Ok(Json(results))
}

// -- Workspace Quota --

#[derive(serde::Serialize)]
pub struct WorkspaceQuotaResponse {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// Pure logic: query the workspace_fs quota for `user_id`. No I/O or DB calls.
pub fn workspace_quota_handler(state: &AppState, user_id: &str) -> WorkspaceQuotaResponse {
    let snap = state.workspace_fs.quota(user_id);
    WorkspaceQuotaResponse {
        used_bytes: snap.used_bytes,
        total_bytes: snap.limit_bytes,
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

/// Map a `WorkspaceError` (from `plexus_common`) to an `ApiError`.
fn map_ws_err(e: plexus_common::errors::workspace::WorkspaceError) -> ApiError {
    use plexus_common::errors::workspace::WorkspaceError;
    match &e {
        WorkspaceError::Traversal(_) => ApiError::new(ErrorCode::Forbidden, "forbidden"),
        WorkspaceError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            ApiError::new(ErrorCode::NotFound, "not found")
        }
        WorkspaceError::UploadTooLarge { .. } | WorkspaceError::SoftLocked => {
            ApiError::new(ErrorCode::ValidationFailed, format!("{e}"))
        }
        _ => ApiError::new(ErrorCode::InternalError, format!("{e}")),
    }
}

async fn workspace_file_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Result<Response<Body>, ApiError> {
    let c = claims(&headers, &state)?;
    let stat = state.workspace_fs.stat(&c.sub, &path).await.map_err(map_ws_err)?;
    let stream = state.workspace_fs.read_stream(&c.sub, &path).await.map_err(map_ws_err)?;
    let body = Body::from_stream(stream);
    let mut resp = Response::new(body);
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        stat.mime.parse().unwrap_or_else(|_| {
            "application/octet-stream".parse().expect("static str parses")
        }),
    );
    resp.headers_mut().insert(
        axum::http::header::CONTENT_LENGTH,
        stat.size.to_string().parse().expect("u64 parses as header value"),
    );
    resp.headers_mut().insert(
        "X-Content-Type-Options",
        "nosniff".parse().expect("static str parses"),
    );
    Ok(resp)
}

async fn workspace_file_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(path): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let c = claims(&headers, &state)?;
    state.workspace_fs.write(&c.sub, &path, &body).await.map_err(map_ws_err)?;
    Ok(StatusCode::NO_CONTENT)
}

// -- Workspace File Delete --

#[derive(serde::Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    pub recursive: bool,
}

async fn workspace_file_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(path): axum::extract::Path<String>,
    Query(q): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    let c = claims(&headers, &state)?;
    state.workspace_fs.delete_path(&c.sub, &path, q.recursive).await.map_err(map_ws_err)?;
    Ok(StatusCode::NO_CONTENT)
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
            "/api/workspace/files/{*path}",
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
    async fn test_workspace_upload_saves_to_uploads_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("alice/uploads"))
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);
        let result = workspace_upload_save_one(&state, "alice", "photo.jpg", b"fakedata".to_vec())
            .await;

        let u = match result.outcome {
            UploadOutcome::Success(u) => u,
            other => panic!("expected Success, got {:?}", other),
        };

        assert!(
            u.path.starts_with("uploads/"),
            "expected uploads/ prefix; got {}",
            u.path
        );
        assert!(u.path.ends_with("photo.jpg"));
        assert_eq!(u.size_bytes, 8);

        // File actually exists on disk.
        let full = tmp.path().join("alice").join(&u.path);
        assert!(full.exists());
    }

    #[tokio::test]
    async fn test_workspace_upload_returns_typed_error_on_oversized() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        // 1 MB quota → 800 KB per-upload cap. Push 900 KB.
        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);
        let big = vec![0u8; 900 * 1024];

        let result = workspace_upload_save_one(&state, "alice", "big.bin", big).await;
        assert_eq!(result.filename, "big.bin");
        match result.outcome {
            UploadOutcome::Error(UploadError::TooLarge) => {}
            other => panic!("expected TooLarge, got {:?}", other),
        }
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
