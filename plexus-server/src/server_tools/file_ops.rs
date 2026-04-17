//! Server-workspace file operations: read_file.

use crate::state::AppState;
use crate::workspace::{resolve_user_path, WorkspaceError};
use serde_json::Value;
use std::sync::Arc;

/// Maximum bytes returned by `read_file` for text content.
/// Files larger than this return a size hint pointing to file_transfer.
/// Sized to comfortably fit within any reasonable LLM context budget.
pub(crate) const MAX_READ_BYTES: u64 = 256 * 1024;

pub async fn read_file(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: path".into()),
    };

    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = match resolve_user_path(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(WorkspaceError::Traversal(_)) => return (1, "Path escapes user workspace".into()),
        Err(WorkspaceError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return (1, format!("File not found: {path}"));
        }
        Err(e) => return (1, format!("Resolve error: {e}")),
    };

    // Size cap check before reading
    let size = match tokio::fs::metadata(&resolved).await {
        Ok(m) => m.len(),
        Err(e) => return (1, format!("Stat error: {e}")),
    };
    if size > MAX_READ_BYTES {
        return (
            0,
            format!(
                "[File too large: {size} bytes, max {MAX_READ_BYTES}. Use file_transfer to move it to a client device.]"
            ),
        );
    }

    match tokio::fs::read_to_string(&resolved).await {
        Ok(content) => (0, content),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            // Binary file — return size + hint instead of raw bytes
            let meta = tokio::fs::metadata(&resolved).await.ok();
            let size = meta.map(|m| m.len()).unwrap_or(0);
            (
                0,
                format!(
                    "[Binary file, {size} bytes. Use file_transfer to move to a client device.]"
                ),
            )
        }
        Err(e) => (1, format!("Read error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_read_file_happy_path() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("hello.txt"), b"hello\n")
            .await
            .unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "hello.txt"});
        let (code, out) = read_file(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(out, "hello\n");
    }

    #[tokio::test]
    async fn test_read_file_traversal_rejected() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let other = tmp.path().join("bob");
        tokio::fs::create_dir_all(&other).await.unwrap();
        tokio::fs::write(other.join("secret"), b"s").await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "../bob/secret"});
        let (code, out) = read_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("escapes"));
    }

    #[tokio::test]
    async fn test_read_file_missing_file_errors() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "nonexistent.txt"});
        let (code, out) = read_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("not found"), "expected 'not found' in output, got: {out}");
        assert!(out.contains("nonexistent.txt"), "expected path echo in output, got: {out}");
    }

    #[tokio::test]
    async fn test_read_file_oversize_returns_stub() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let large = vec![b'x'; (MAX_READ_BYTES + 1) as usize];
        tokio::fs::write(user_dir.join("big.txt"), &large).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "big.txt"});
        let (code, out) = read_file(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert!(out.contains("too large"), "expected 'too large' in output, got: {out}");
        assert!(out.contains("file_transfer"), "expected remediation hint, got: {out}");
    }
}
