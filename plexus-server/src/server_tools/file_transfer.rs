//! file_transfer server tool: relay files between devices.

use crate::state::AppState;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

pub async fn file_transfer(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let from_device = match args.get("from_device").and_then(Value::as_str) {
        Some(d) => d,
        None => return (1, "Missing required parameter: from_device".into()),
    };
    let to_device = match args.get("to_device").and_then(Value::as_str) {
        Some(d) => d,
        None => return (1, "Missing required parameter: to_device".into()),
    };
    let file_path = match args.get("file_path").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: file_path".into()),
    };

    // Step 1: Get file content from source
    let (content_bytes, filename) = if from_device == "server" {
        // Read from server filesystem (relative to user workspace)
        match read_server_file(state.as_ref(), user_id, file_path).await {
            Ok(data) => data,
            Err(e) => return (1, e),
        }
    } else {
        // Request from client device
        match request_file_from_device(state, user_id, from_device, file_path).await {
            Ok(data) => data,
            Err(e) => return (1, e),
        }
    };

    // Step 2: Send file to destination
    if to_device == "server" {
        // Derive the filename from the source path.
        let fname = Path::new(file_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if fname.is_empty() {
            return (1, "Cannot derive a filename from file_path".into());
        }

        let rel_target = format!("uploads/{fname}");
        let new_size = content_bytes.len() as u64;

        let ws_root = std::path::Path::new(&state.config.workspace_root);
        let target =
            match crate::workspace::resolve_user_path_for_create(ws_root, user_id, &rel_target)
                .await
            {
                Ok(p) => p,
                Err(crate::workspace::WorkspaceError::Traversal(_)) => {
                    return (1, "Path escapes user workspace".into());
                }
                Err(crate::workspace::WorkspaceError::Io(e))
                    if e.kind() == std::io::ErrorKind::NotFound =>
                {
                    return (1, format!("Parent path not found: uploads/{fname}"));
                }
                Err(e) => return (1, format!("Resolve error: {e}")),
            };

        // Net-delta quota accounting: stat existing file first to avoid double-counting.
        let old_size = tokio::fs::metadata(&target)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let growth = new_size.saturating_sub(old_size);

        if growth > 0 {
            if let Err(e) = state.quota.check_and_reserve_upload(user_id, growth) {
                return (1, format!("Quota: {e}"));
            }
        }

        // Ensure uploads/ dir exists.
        if let Some(parent) = target.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                if growth > 0 {
                    state.quota.record_delete(user_id, growth);
                }
                return (1, format!("Create dir: {e}"));
            }
        }

        if let Err(e) = tokio::fs::write(&target, &content_bytes).await {
            if growth > 0 {
                state.quota.record_delete(user_id, growth);
            }
            return (1, format!("Write error: {e}"));
        }

        // If the new file is smaller than the old one, release the freed bytes.
        let shrink = old_size.saturating_sub(new_size);
        if shrink > 0 {
            state.quota.record_delete(user_id, shrink);
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(
                &target,
                std::fs::Permissions::from_mode(0o600),
            )
            .await;
        }

        (0, format!("File saved to server: {rel_target}"))
    } else {
        // Send to client device — unchanged.
        match send_file_to_device(state, user_id, to_device, &filename, &content_bytes).await {
            Ok(()) => (0, format!("File transferred: {from_device} -> {to_device}")),
            Err(e) => (1, e),
        }
    }
}

/// Read a file from the user's server workspace. `file_path` is relative to
/// the user workspace root (e.g. `"uploads/report.pdf"`, `"skills/git/SKILL.md"`).
/// Returns (bytes, canonical_filename).
async fn read_server_file(
    state: &AppState,
    user_id: &str,
    file_path: &str,
) -> Result<(Vec<u8>, String), String> {
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path(ws_root, user_id, file_path)
        .await
        .map_err(|e| match e {
            crate::workspace::WorkspaceError::Traversal(_) => {
                "Path escapes user workspace".to_string()
            }
            crate::workspace::WorkspaceError::Io(e)
                if e.kind() == std::io::ErrorKind::NotFound =>
            {
                format!("File not found: {file_path}")
            }
            e => format!("Resolve error: {e}"),
        })?;

    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Read file: {e}"))?;
    let filename = resolved
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok((data, filename))
}

/// Request a file from a client device via FileRequest protocol.
pub async fn request_file_from_device(
    state: &AppState,
    user_id: &str,
    device_name: &str,
    file_path: &str,
) -> Result<(Vec<u8>, String), String> {
    let device_key = AppState::device_key(user_id, device_name);
    let conn = state
        .devices
        .get(&device_key)
        .ok_or_else(|| format!("Device '{device_name}' is offline"))?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .pending
        .entry(device_key.clone())
        .or_default()
        .insert(request_id.clone(), tx);

    let msg = plexus_common::protocol::ServerToClient::FileRequest {
        request_id: request_id.clone(),
        path: file_path.to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    {
        let mut sink = conn.sink.lock().await;
        futures_util::SinkExt::send(&mut *sink, axum::extract::ws::Message::Text(json.into()))
            .await
            .map_err(|e| format!("Send FileRequest: {e}"))?;
    }
    drop(conn);

    let result = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
        .await
        .map_err(|_| "File request timed out".to_string())?
        .map_err(|_| format!("Device '{device_name}' disconnected"))?;

    if result.exit_code != 0 {
        return Err(format!("File request failed: {}", result.output));
    }

    let file_data: Value =
        serde_json::from_str(&result.output).map_err(|e| format!("Parse response: {e}"))?;
    let b64 = file_data
        .get("content_base64")
        .and_then(Value::as_str)
        .ok_or("Missing content_base64 in response")?;

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("Decode base64: {e}"))?;

    let filename = Path::new(file_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    Ok((bytes, filename))
}

/// Send a file to a client device via FileSend protocol.
async fn send_file_to_device(
    state: &AppState,
    user_id: &str,
    device_name: &str,
    filename: &str,
    content: &[u8],
) -> Result<(), String> {
    let device_key = AppState::device_key(user_id, device_name);
    let conn = state
        .devices
        .get(&device_key)
        .ok_or_else(|| format!("Device '{device_name}' is offline"))?;

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(content);
    let request_id = uuid::Uuid::new_v4().to_string();

    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .pending
        .entry(device_key.clone())
        .or_default()
        .insert(request_id.clone(), tx);

    let msg = plexus_common::protocol::ServerToClient::FileSend {
        request_id: request_id.clone(),
        filename: filename.to_string(),
        content_base64: b64,
        destination: filename.to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    {
        let mut sink = conn.sink.lock().await;
        futures_util::SinkExt::send(&mut *sink, axum::extract::ws::Message::Text(json.into()))
            .await
            .map_err(|e| format!("Send FileSend: {e}"))?;
    }
    drop(conn);

    let result = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
        .await
        .map_err(|_| "File send timed out".to_string())?
        .map_err(|_| format!("Device '{device_name}' disconnected"))?;

    if result.exit_code != 0 {
        return Err(format!("File send failed: {}", result.output));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_read_server_file_happy_path() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_dir.join("uploads")).await.unwrap();
        tokio::fs::write(user_dir.join("uploads/a.txt"), b"hello").await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let (bytes, fname) = read_server_file(state.as_ref(), "alice", "uploads/a.txt")
            .await
            .unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(fname, "a.txt");
    }

    #[tokio::test]
    async fn test_read_server_file_traversal_rejected() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let bob = tmp.path().join("bob");
        tokio::fs::create_dir_all(&bob).await.unwrap();
        tokio::fs::write(bob.join("secret.txt"), b"s").await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let err = read_server_file(state.as_ref(), "alice", "../bob/secret.txt")
            .await
            .unwrap_err();
        assert!(err.contains("escapes"), "got: {err}");
    }

    #[tokio::test]
    async fn test_read_server_file_missing_errors() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let err = read_server_file(state.as_ref(), "alice", "ghost.txt")
            .await
            .unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[tokio::test]
    async fn test_file_transfer_server_to_server_write_with_quota() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_dir.join("src")).await.unwrap();
        tokio::fs::write(user_dir.join("src/x.txt"), b"contents").await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let args = serde_json::json!({
            "from_device": "server",
            "to_device": "server",
            "file_path": "src/x.txt",
        });
        let (code, out) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0, "got: {out}");
        assert!(out.contains("uploads/x.txt"), "got: {out}");
        assert!(user_dir.join("uploads/x.txt").exists());
        assert_eq!(state.quota.current_usage("alice"), 8); // "contents" = 8 bytes
    }

    #[tokio::test]
    async fn test_file_transfer_server_to_server_overwrite_tracks_net_size() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_dir.join("src")).await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());

        // First transfer: 5 bytes.
        tokio::fs::write(user_dir.join("src/x.txt"), b"hello").await.unwrap();
        let args = serde_json::json!({
            "from_device": "server",
            "to_device": "server",
            "file_path": "src/x.txt",
        });
        let (code, _) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 5);

        // Second transfer: grow to 10 bytes.
        tokio::fs::write(user_dir.join("src/x.txt"), b"helloworld").await.unwrap();
        let (code, _) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 10);

        // Third transfer: shrink to 3 bytes.
        tokio::fs::write(user_dir.join("src/x.txt"), b"hey").await.unwrap();
        let (code, _) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 3);
    }
}
