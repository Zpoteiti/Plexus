//! File upload/download with user-isolated paths in the workspace.
//! Persistent storage — the ephemeral 24h cleanup from the old /tmp-based
//! file store was dropped in A-18 since the workspace quota system
//! handles size bounds and files are durable until the user deletes them.

use crate::state::AppState;
use plexus_common::error::{ApiError, ErrorCode};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tracing::warn;

pub fn user_upload_dir(workspace_root: &std::path::Path, user_id: &str) -> PathBuf {
    workspace_root.join(user_id).join("uploads")
}

pub async fn save_upload(
    state: &Arc<AppState>,
    user_id: &str,
    filename: &str,
    data: &[u8],
) -> Result<String, ApiError> {
    let size = data.len() as u64;

    // Quota check via the central QuotaCache (per-upload cap + soft-lock).
    state
        .quota
        .check_and_reserve_upload(user_id, size)
        .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, format!("{e}")))?;

    let file_id = uuid::Uuid::new_v4().to_string();
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let dir = user_upload_dir(ws_root, user_id);

    if let Err(e) = fs::create_dir_all(&dir).await {
        state.quota.record_delete(user_id, size); // rollback
        return Err(ApiError::new(
            ErrorCode::InternalError,
            format!("mkdir: {e}"),
        ));
    }

    let safe_name = sanitize_filename(filename);
    let path = dir.join(format!("{file_id}_{safe_name}"));
    if let Err(e) = fs::write(&path, data).await {
        state.quota.record_delete(user_id, size); // rollback
        return Err(ApiError::new(
            ErrorCode::InternalError,
            format!("write: {e}"),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await {
            tracing::warn!(error = %e, path = %path.display(), "failed to set 0600 perms on uploaded file");
        }
    }

    Ok(file_id)
}

pub async fn load_file(
    state: &Arc<AppState>,
    user_id: &str,
    file_id: &str,
) -> Result<(Vec<u8>, String), ApiError> {
    if file_id.contains("..") || file_id.contains('/') || file_id.contains('\\') {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            "Invalid file ID",
        ));
    }
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let dir = user_upload_dir(ws_root, user_id);
    let mut entries = fs::read_dir(&dir)
        .await
        .map_err(|_| ApiError::new(ErrorCode::NotFound, "No files found"))?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(file_id) {
            let data = fs::read(entry.path())
                .await
                .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("read: {e}")))?;
            let original_name = name
                .strip_prefix(&format!("{file_id}_"))
                .unwrap_or(&name)
                .to_string();
            return Ok((data, original_name));
        }
    }
    Err(ApiError::new(ErrorCode::NotFound, "File not found"))
}

/// A resolved media item ready for channel delivery.
pub enum ResolvedMedia {
    /// File bytes + filename, fetched from the server file store.
    File { bytes: Vec<u8>, filename: String },
    /// Raw URL/path passed through as-is (e.g. direct URLs from future channels).
    Url(String),
}

/// Resolve media paths for channel delivery. Loads all `/api/files/{id}` entries
/// in parallel; passes through anything else as a raw URL.
pub async fn resolve_media(
    state: &Arc<AppState>,
    user_id: &str,
    media_paths: &[String],
) -> Vec<ResolvedMedia> {
    let futures = media_paths.iter().map(|path| {
        let path = path.clone();
        let user_id = user_id.to_string();
        let state = state.clone();
        async move {
            if let Some(file_id) = path.strip_prefix("/api/files/") {
                match load_file(&state, &user_id, file_id).await {
                    Ok((bytes, filename)) => ResolvedMedia::File { bytes, filename },
                    Err(e) => {
                        warn!("resolve_media: failed to load {file_id}: {}", e.message);
                        ResolvedMedia::Url(path)
                    }
                }
            } else {
                ResolvedMedia::Url(path)
            }
        }
    });
    futures_util::future::join_all(futures).await
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
