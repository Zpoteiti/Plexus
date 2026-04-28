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
        // Read from server workspace via WorkspaceFs
        let bytes = match state.workspace_fs.read(user_id, file_path).await {
            Ok(b) => b,
            Err(e) => return (1, workspace_err_to_string(e, file_path)),
        };
        let fname = Path::new(file_path)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("file.bin"))
            .to_string_lossy()
            .to_string();
        (bytes, fname)
    } else {
        // Request from client device (with retry)
        match with_retry("request_file_from_device", || {
            request_file_from_device(state, user_id, from_device, file_path)
        })
        .await
        {
            Ok(data) => data,
            Err(e) => return (1, e),
        }
    };

    // Step 2: Send file to destination
    if to_device == "server" {
        if filename.is_empty() {
            return (1, "Cannot derive a filename from file_path".into());
        }

        let rel_target = format!("uploads/{filename}");

        match state
            .workspace_fs
            .write(user_id, &rel_target, &content_bytes)
            .await
        {
            Ok(()) => (0, format!("File saved to server: {rel_target}")),
            Err(e) => (1, workspace_err_to_string(e, &rel_target)),
        }
    } else {
        // Send to client device (with retry)
        match with_retry("send_file_to_device", || {
            send_file_to_device(state, user_id, to_device, &filename, &content_bytes)
        })
        .await
        {
            Ok(()) => (0, format!("File transferred: {from_device} -> {to_device}")),
            Err(e) => (1, e),
        }
    }
}

/// Map WorkspaceError to a user-facing string.
fn workspace_err_to_string(
    e: plexus_common::errors::workspace::WorkspaceError,
    path: &str,
) -> String {
    match e {
        plexus_common::errors::workspace::WorkspaceError::Traversal(_) => {
            "Path escapes user workspace".to_string()
        }
        plexus_common::errors::workspace::WorkspaceError::Io(io)
            if io.kind() == std::io::ErrorKind::NotFound =>
        {
            format!("File not found: {path}")
        }
        plexus_common::errors::workspace::WorkspaceError::UploadTooLarge { actual, limit } => {
            format!("Quota: upload too large ({actual} bytes; cap {limit} bytes)")
        }
        plexus_common::errors::workspace::WorkspaceError::SoftLocked => {
            "Quota: workspace is soft-locked; delete files to continue".to_string()
        }
        plexus_common::errors::workspace::WorkspaceError::Io(io) => format!("IO error: {io}"),
    }
}

/// Retry wrapper: 3 attempts with exponential backoff (500ms, 1s, 2s).
async fn with_retry<T, F, Fut>(label: &str, f: F) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut delay = std::time::Duration::from_millis(500);
    for attempt in 0..3u32 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt == 2 => return Err(format!("{label} failed after 3 attempts: {e}")),
            Err(_) => tokio::time::sleep(delay).await,
        }
        delay *= 2;
    }
    unreachable!()
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
    async fn file_transfer_server_to_server_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_dir.join("src"))
            .await
            .unwrap();
        tokio::fs::write(user_dir.join("src/hello.txt"), b"roundtrip")
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let args = serde_json::json!({
            "from_device": "server",
            "to_device": "server",
            "file_path": "src/hello.txt",
        });
        let (code, out) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0, "got: {out}");
        assert!(out.contains("uploads/hello.txt"), "got: {out}");
        assert!(user_dir.join("uploads/hello.txt").exists());
        let written = tokio::fs::read(user_dir.join("uploads/hello.txt"))
            .await
            .unwrap();
        assert_eq!(written, b"roundtrip");
    }

    #[tokio::test]
    async fn test_file_transfer_server_to_server_write_with_quota() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_dir.join("src"))
            .await
            .unwrap();
        tokio::fs::write(user_dir.join("src/x.txt"), b"contents")
            .await
            .unwrap();

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
        tokio::fs::create_dir_all(user_dir.join("src"))
            .await
            .unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());

        // First transfer: 5 bytes.
        tokio::fs::write(user_dir.join("src/x.txt"), b"hello")
            .await
            .unwrap();
        let args = serde_json::json!({
            "from_device": "server",
            "to_device": "server",
            "file_path": "src/x.txt",
        });
        let (code, _) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 5);

        // Second transfer: grow to 10 bytes.
        tokio::fs::write(user_dir.join("src/x.txt"), b"helloworld")
            .await
            .unwrap();
        let (code, _) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 10);

        // Third transfer: shrink to 3 bytes.
        tokio::fs::write(user_dir.join("src/x.txt"), b"hey")
            .await
            .unwrap();
        let (code, _) = file_transfer(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 3);
    }
}
