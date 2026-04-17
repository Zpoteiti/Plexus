//! Server-workspace file operations: read_file, write_file, edit_file.

use crate::state::AppState;
use crate::workspace::{
    is_under_skills_dir, resolve_user_path, resolve_user_path_for_create, WorkspaceError,
};
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

pub async fn write_file(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: path".into()),
    };
    let content = match args.get("content").and_then(Value::as_str) {
        Some(c) => c,
        None => return (1, "Missing required parameter: content".into()),
    };

    let bytes = content.as_bytes();
    let new_size = bytes.len() as u64;

    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = match resolve_user_path_for_create(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(WorkspaceError::Traversal(_)) => {
            return (1, "Path escapes user workspace".into());
        }
        Err(WorkspaceError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return (1, format!("Parent path not found: {path}"));
        }
        Err(e) => return (1, format!("Resolve error: {e}")),
    };

    let old_size = tokio::fs::metadata(&resolved)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    // If the new write grows the file, reserve the growth up-front.
    // (This is the only point where the quota check can reject us.)
    let growth = new_size.saturating_sub(old_size);
    if growth > 0 {
        if let Err(e) = state.quota.check_and_reserve_upload(user_id, growth) {
            return (1, format!("Quota: {e}"));
        }
    }

    // Make parent dirs if needed.
    if let Some(parent) = resolved.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            if growth > 0 {
                // Infallible rollback: release what we reserved.
                state.quota.record_delete(user_id, growth);
            }
            return (1, format!("Create dir: {e}"));
        }
    }

    // Write.
    if let Err(e) = tokio::fs::write(&resolved, bytes).await {
        if growth > 0 {
            state.quota.record_delete(user_id, growth);
        }
        return (1, format!("Write error: {e}"));
    }

    // Write succeeded. If the new file is smaller than the old, release the excess.
    let shrink = old_size.saturating_sub(new_size);
    if shrink > 0 {
        state.quota.record_delete(user_id, shrink);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(0o600)).await;
    }

    // Invalidate the skills cache if this write landed under `skills/`.
    // Check against the RESOLVED path (not raw input) so `./skills/foo`,
    // `skills//foo`, etc. all invalidate correctly.
    if is_under_skills_dir(&resolved, ws_root, user_id) {
        state.skills_cache.invalidate(user_id);
    }

    (0, format!("Wrote {new_size} bytes to {path}"))
}


pub async fn edit_file(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: path".into()),
    };
    let old_string = match args.get("old_string").and_then(Value::as_str) {
        Some(s) => s,
        None => return (1, "Missing required parameter: old_string".into()),
    };
    let new_string = match args.get("new_string").and_then(Value::as_str) {
        Some(s) => s,
        None => return (1, "Missing required parameter: new_string".into()),
    };

    if old_string.is_empty() {
        return (1, "old_string must not be empty".into());
    }

    let ws_root = std::path::Path::new(&state.config.workspace_root);

    // File must already exist to edit.
    let resolved = match resolve_user_path(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(WorkspaceError::Traversal(_)) => return (1, "Path escapes user workspace".into()),
        Err(WorkspaceError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return (1, format!("File not found: {path}"));
        }
        Err(e) => return (1, format!("Resolve error: {e}")),
    };

    // Read existing content. Enforce the same 256 KiB cap as read_file.
    let old_size = match tokio::fs::metadata(&resolved).await {
        Ok(m) => m.len(),
        Err(e) => return (1, format!("Stat error: {e}")),
    };
    if old_size > MAX_READ_BYTES {
        return (
            1,
            format!(
                "File too large to edit: {old_size} bytes, max {MAX_READ_BYTES}. Use file_transfer to move it to a client device for larger edits."
            ),
        );
    }

    let current = match tokio::fs::read_to_string(&resolved).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return (
                1,
                "Cannot edit: file is not valid UTF-8 (binary or non-UTF-8 text).".into(),
            );
        }
        Err(e) => return (1, format!("Read error: {e}")),
    };

    // Unique-match check.
    let match_count = current.matches(old_string).count();
    if match_count == 0 {
        return (
            1,
            format!(
                "old_string not found in {path}. Include surrounding context to disambiguate if the target appears elsewhere."
            ),
        );
    }
    if match_count > 1 {
        return (
            1,
            format!(
                "old_string appears {match_count} times in {path}. Include more surrounding context to make the match unique."
            ),
        );
    }

    let updated = current.replacen(old_string, new_string, 1);
    let new_size = updated.len() as u64;

    // Net-delta quota accounting (A-8 pattern).
    let growth = new_size.saturating_sub(old_size);
    if growth > 0 {
        if let Err(e) = state.quota.check_and_reserve_upload(user_id, growth) {
            return (1, format!("Quota: {e}"));
        }
    }

    if let Err(e) = tokio::fs::write(&resolved, &updated).await {
        if growth > 0 {
            state.quota.record_delete(user_id, growth);
        }
        return (1, format!("Write error: {e}"));
    }

    let shrink = old_size.saturating_sub(new_size);
    if shrink > 0 {
        state.quota.record_delete(user_id, shrink);
    }

    if is_under_skills_dir(&resolved, ws_root, user_id) {
        state.skills_cache.invalidate(user_id);
    }

    (0, format!("Edited {path} ({old_size} → {new_size} bytes)"))
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

    // --- write_file tests ---

    #[tokio::test]
    async fn test_write_file_happy_path() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "hello.txt", "content": "hi"});
        let (code, out) = write_file(&state, "alice", &args).await;
        assert_eq!(code, 0, "got: {out}");
        assert_eq!(
            tokio::fs::read_to_string(user_dir.join("hello.txt")).await.unwrap(),
            "hi"
        );
        assert_eq!(state.quota.current_usage("alice"), 2);
    }

    #[tokio::test]
    async fn test_write_file_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "a/b/c/x.txt", "content": "hi"});
        let (code, _) = write_file(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert!(user_dir.join("a/b/c/x.txt").exists());
    }

    #[tokio::test]
    async fn test_write_file_traversal_rejected() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "../evil.txt", "content": "x"});
        let (code, out) = write_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("escapes"));
    }

    #[tokio::test]
    async fn test_write_file_over_per_upload_cap() {
        // Use a tiny 1000-byte quota: per-upload cap = 800 bytes.
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal_with_quota(tmp.path(), 1000);
        let content = "x".repeat(900); // 900 > 800 cap
        let args = serde_json::json!({"path": "big.txt", "content": content});
        let (code, out) = write_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.to_lowercase().contains("quota"), "got: {out}");
    }

    #[tokio::test]
    async fn test_write_file_overwrite_tracks_net_size_change() {
        // Verify shrink overwrite correctly reduces quota usage.
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());

        // Write 100 bytes
        let big = "x".repeat(100);
        let args = serde_json::json!({"path": "f.txt", "content": big});
        write_file(&state, "alice", &args).await;
        assert_eq!(state.quota.current_usage("alice"), 100);

        // Overwrite with 10 bytes — usage should drop to 10, not grow to 110.
        let small = "x".repeat(10);
        let args = serde_json::json!({"path": "f.txt", "content": small});
        write_file(&state, "alice", &args).await;
        assert_eq!(state.quota.current_usage("alice"), 10);
    }

    #[tokio::test]
    async fn test_write_file_rollback_on_failure_restores_quota() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());

        // Write a file successfully (100 bytes).
        let args1 = serde_json::json!({"path": "f.txt", "content": "x".repeat(100)});
        let (code, _) = write_file(&state, "alice", &args1).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 100);

        // Attempt a grow-overwrite into a read-only directory — will fail at mkdir.
        // Create a file named "readonly_dir" (so mkdir will fail because it exists
        // but is a file, not a directory).
        tokio::fs::write(user_dir.join("readonly_dir"), b"block").await.unwrap();
        let args2 = serde_json::json!({
            "path": "readonly_dir/child.txt",
            "content": "y".repeat(200),  // growth
        });
        let (code, out) = write_file(&state, "alice", &args2).await;
        assert_eq!(code, 1, "expected failure, got code {code} out {out}");

        // Quota should be unchanged — the attempted 200-byte reservation was rolled back.
        // Correct usage is still 100 (from the first write).
        assert_eq!(
            state.quota.current_usage("alice"),
            100,
            "quota should be unchanged after failed grow; got {}",
            state.quota.current_usage("alice")
        );
    }

    #[tokio::test]
    async fn test_write_file_grow_within_quota() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());

        // 50 bytes
        let args1 = serde_json::json!({"path": "f.txt", "content": "x".repeat(50)});
        write_file(&state, "alice", &args1).await;
        assert_eq!(state.quota.current_usage("alice"), 50);

        // Grow to 90 bytes.
        let args2 = serde_json::json!({"path": "f.txt", "content": "x".repeat(90)});
        write_file(&state, "alice", &args2).await;
        assert_eq!(state.quota.current_usage("alice"), 90);
    }

    // --- edit_file tests ---

    #[tokio::test]
    async fn test_edit_file_unique_match_succeeds() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("m.txt"), "hello world\nfoo bar\n")
            .await
            .unwrap();

        let state = AppState::test_minimal(tmp.path());
        // Prime the quota so shrink/grow accounting is exercised.
        state.quota.check_and_reserve_upload("alice", 19).unwrap(); // matches file size

        let args = serde_json::json!({
            "path": "m.txt",
            "old_string": "foo bar",
            "new_string": "foo baz"
        });
        let (code, _) = edit_file(&state, "alice", &args).await;
        assert_eq!(code, 0);
        let after = tokio::fs::read_to_string(user_dir.join("m.txt")).await.unwrap();
        assert!(after.contains("foo baz"));
        assert!(!after.contains("foo bar"));
        // Same size edit — quota unchanged.
        assert_eq!(state.quota.current_usage("alice"), 19);
    }

    #[tokio::test]
    async fn test_edit_file_no_match_errors() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("m.txt"), "nothing here").await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({
            "path": "m.txt",
            "old_string": "missing",
            "new_string": "replaced"
        });
        let (code, out) = edit_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("not found"), "got: {out}");
    }

    #[tokio::test]
    async fn test_edit_file_ambiguous_match_errors() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("m.txt"), "abc\nabc\n").await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "m.txt", "old_string": "abc", "new_string": "xyz"});
        let (code, out) = edit_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("appears 2 times") || out.contains("2 times"), "got: {out}");
    }

    #[tokio::test]
    async fn test_edit_file_missing_file_errors() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({"path": "ghost.txt", "old_string": "x", "new_string": "y"});
        let (code, out) = edit_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("File not found"), "got: {out}");
        assert!(out.contains("ghost.txt"), "expected path echo, got: {out}");
    }

    #[tokio::test]
    async fn test_edit_file_grow_tracks_quota() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("f.txt"), "hi").await.unwrap(); // 2 bytes

        let state = AppState::test_minimal(tmp.path());
        state.quota.check_and_reserve_upload("alice", 2).unwrap();
        assert_eq!(state.quota.current_usage("alice"), 2);

        let args = serde_json::json!({
            "path": "f.txt",
            "old_string": "hi",
            "new_string": "hello world"  // 11 bytes; grow by 9
        });
        let (code, _) = edit_file(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 11);
    }

    #[tokio::test]
    async fn test_edit_file_shrink_tracks_quota() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("f.txt"), "hello world").await.unwrap(); // 11 bytes

        let state = AppState::test_minimal(tmp.path());
        state.quota.check_and_reserve_upload("alice", 11).unwrap();

        let args = serde_json::json!({
            "path": "f.txt",
            "old_string": "hello world",
            "new_string": "hi"  // 2 bytes; shrink by 9
        });
        let (code, _) = edit_file(&state, "alice", &args).await;
        assert_eq!(code, 0);
        assert_eq!(state.quota.current_usage("alice"), 2);
    }

    #[tokio::test]
    async fn test_edit_file_too_large_rejected() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let large = vec![b'x'; (MAX_READ_BYTES + 1) as usize];
        tokio::fs::write(user_dir.join("big.txt"), &large).await.unwrap();

        let state = AppState::test_minimal(tmp.path());
        let args = serde_json::json!({
            "path": "big.txt",
            "old_string": "xxx",
            "new_string": "yyy",
        });
        let (code, out) = edit_file(&state, "alice", &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("too large"), "got: {out}");
    }
}
