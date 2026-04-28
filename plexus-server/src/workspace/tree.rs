//! User workspace tree enumeration for the Workspace page (Plan B).

use serde::Serialize;
use std::path::Path;
use tracing::warn;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct WorkspaceEntry {
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_at: chrono::DateTime<chrono::Utc>,
}

/// Walk the user's workspace tree depth-first. Returns a flat sorted list
/// of entries, paths relative to `{user_root}`. Symlinks are followed but
/// their targets must still live under `{user_root}` (canonicalized prefix
/// check — same invariant as `resolve_user_path`).
pub async fn walk_user_tree(
    workspace_root: &Path,
    user_id: &str,
) -> std::io::Result<Vec<WorkspaceEntry>> {
    let user_root = workspace_root.join(user_id);
    let user_root_canon = tokio::fs::canonicalize(&user_root).await?;

    let user_root_for_task = user_root_canon.clone();
    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(&user_root_for_task)
            .follow_links(true)
            .min_depth(1)
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue, // broken symlinks, permission errors — skip
            };
            let full = entry.path();
            // Symlink escape check: canonicalize and ensure still under user_root.
            let canon = match full.canonicalize() {
                Ok(c) => c,
                Err(_) => continue,
            };
            if !canon.starts_with(&user_root_for_task) {
                warn!(
                    path = %full.display(),
                    "workspace tree: symlink escape blocked"
                );
                continue;
            }
            let rel = match full.strip_prefix(&user_root_for_task) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(chrono::DateTime::<chrono::Utc>::from)
                .unwrap_or_else(chrono::Utc::now);
            entries.push(WorkspaceEntry {
                path: rel.to_string_lossy().into_owned(),
                is_dir: entry.file_type().is_dir(),
                size_bytes: if entry.file_type().is_dir() { 0 } else { size },
                modified_at: modified,
            });
        }
        // Directories first, then alphabetical.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path.cmp(&b.path),
        });
        Ok(entries)
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::other(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn walk_returns_sorted_entries() {
        let tmp = TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_root.join("skills/foo"))
            .await
            .unwrap();
        tokio::fs::write(user_root.join("SOUL.md"), b"hello")
            .await
            .unwrap();
        tokio::fs::write(
            user_root.join("skills/foo/SKILL.md"),
            b"---\nname: foo\n---",
        )
        .await
        .unwrap();

        let entries = walk_user_tree(tmp.path(), "alice").await.unwrap();

        // Directories first.
        assert!(
            entries[0].is_dir,
            "expected first entry to be dir; got {:?}",
            entries
        );
        // All paths relative.
        for e in &entries {
            assert!(
                !e.path.starts_with('/'),
                "paths must be relative; got {}",
                e.path
            );
        }
        // SOUL.md present with its bytes.
        let soul = entries
            .iter()
            .find(|e| e.path == "SOUL.md")
            .expect("SOUL.md missing");
        assert_eq!(soul.size_bytes, 5);
        assert!(!soul.is_dir);
    }

    #[tokio::test]
    async fn walk_rejects_symlink_escape() {
        let tmp = TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        let outside = tmp.path().join("outside.txt");
        tokio::fs::write(&outside, b"secret").await.unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, user_root.join("escape.txt")).unwrap();

        let entries = walk_user_tree(tmp.path(), "alice").await.unwrap();

        // escape.txt must NOT be in the results (its canonicalized target escapes user_root).
        assert!(
            !entries.iter().any(|e| e.path == "escape.txt"),
            "symlink escape leaked into walk output: {:?}",
            entries
        );
    }
}
