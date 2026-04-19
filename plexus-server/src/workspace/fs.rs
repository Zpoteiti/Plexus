//! Workspace filesystem service.
//!
//! `WorkspaceFs` is the single entry point for all per-user file I/O.
//! It enforces path traversal checks, quota, and skills-cache invalidation.
//! Method bodies are stubbed — implementations land in P3.2–P3.5.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use plexus_common::errors::workspace::WorkspaceError;
use tracing::warn;

// ── Supporting types ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct FileStat {
    pub path: String,
    pub size: u64,
    pub mime: String,
    pub mtime: std::time::SystemTime,
}

pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
}

pub enum EntryKind {
    File,
    Dir,
}

#[derive(Default)]
pub struct GrepOpts {
    pub case_insensitive: bool,
    pub context_lines: usize,
    pub file_type: Option<String>,
    pub file_glob: Option<String>,
}

pub struct GrepHit {
    pub path: String,
    pub line_number: u64,
    pub line_content: String,
}

pub struct QuotaSnapshot {
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

// ── WorkspaceFs ───────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct WorkspaceFs {
    root: PathBuf,
    quota: Arc<crate::workspace::quota::QuotaCache>,
    skills_cache: Arc<crate::skills_cache::SkillsCache>,
}

impl WorkspaceFs {
    pub fn new(
        root: PathBuf,
        quota: Arc<crate::workspace::quota::QuotaCache>,
        skills_cache: Arc<crate::skills_cache::SkillsCache>,
    ) -> Self {
        Self {
            root,
            quota,
            skills_cache,
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Resolve `path` (absolute or relative) to a canonical `PathBuf` that is
    /// guaranteed to be inside `<root>/<user_id>/`. Returns
    /// `WorkspaceError::Traversal` on any escape attempt, with a `warn!` log.
    async fn resolve_path(&self, user_id: &str, path: &str) -> Result<PathBuf, WorkspaceError> {
        let result = if Path::new(path).is_absolute() {
            // Canonicalize both sides and do a prefix check.
            let canonical = tokio::fs::canonicalize(path).await.map_err(|e| {
                // A missing file or escape via non-existent path — treat as IO.
                WorkspaceError::Io(e)
            })?;
            let user_root_canonical =
                tokio::fs::canonicalize(self.root.join(user_id)).await?;
            if !canonical.starts_with(&user_root_canonical) {
                Err(WorkspaceError::Traversal(path.into()))
            } else {
                Ok(canonical)
            }
        } else {
            // Delegate to the existing relative-path helper, mapping its error type.
            crate::workspace::paths::resolve_user_path(&self.root, user_id, path)
                .await
                .map_err(|e| match e {
                    crate::workspace::paths::WorkspaceError::Traversal(s) => {
                        WorkspaceError::Traversal(s)
                    }
                    crate::workspace::paths::WorkspaceError::Io(io) => WorkspaceError::Io(io),
                    crate::workspace::paths::WorkspaceError::Quota(_) => {
                        // paths.rs only returns Traversal or Io; this arm is exhaustive.
                        WorkspaceError::Traversal(path.into())
                    }
                })
        };
        if let Err(WorkspaceError::Traversal(_)) = &result {
            warn!(user_id, path, "workspace path escape attempt");
        }
        result
    }

    // ── Reads ─────────────────────────────────────────────────────────────────

    pub async fn read(&self, user_id: &str, path: &str) -> Result<Vec<u8>, WorkspaceError> {
        let resolved = self.resolve_path(user_id, path).await?;
        let bytes = tokio::fs::read(&resolved).await?;
        Ok(bytes)
    }

    pub async fn read_stream(
        &self,
        user_id: &str,
        path: &str,
    ) -> Result<tokio_util::io::ReaderStream<tokio::fs::File>, WorkspaceError> {
        let resolved = self.resolve_path(user_id, path).await?;
        let file = tokio::fs::File::open(&resolved).await?;
        Ok(tokio_util::io::ReaderStream::new(file))
    }

    pub async fn stat(&self, user_id: &str, path: &str) -> Result<FileStat, WorkspaceError> {
        let resolved = self.resolve_path(user_id, path).await?;
        let meta = tokio::fs::metadata(&resolved).await?;
        let size = meta.len();
        let mtime = meta.modified()?;
        let mime = plexus_common::mime::detect_mime_from_extension(
            resolved.to_str().unwrap_or(""),
        )
        .to_owned();
        Ok(FileStat {
            path: path.to_owned(),
            size,
            mime,
            mtime,
        })
    }

    // ── Writes ────────────────────────────────────────────────────────────────

    pub async fn write(
        &self,
        _user_id: &str,
        _path: &str,
        _bytes: &[u8],
    ) -> Result<(), WorkspaceError> {
        unimplemented!()
    }

    pub async fn write_stream<R: tokio::io::AsyncRead + Unpin>(
        &self,
        _user_id: &str,
        _path: &str,
        _reader: R,
        _expected_size: u64,
    ) -> Result<(), WorkspaceError> {
        unimplemented!()
    }

    // ── Deletes ───────────────────────────────────────────────────────────────

    pub async fn delete(&self, _user_id: &str, _path: &str) -> Result<(), WorkspaceError> {
        unimplemented!()
    }

    pub async fn delete_prefix(
        &self,
        _user_id: &str,
        _prefix: &str,
        _older_than: Option<std::time::Duration>,
    ) -> Result<u64, WorkspaceError> {
        unimplemented!()
    }

    // ── Directory / search ────────────────────────────────────────────────────

    pub async fn list(
        &self,
        _user_id: &str,
        _path: &str,
    ) -> Result<Vec<DirEntry>, WorkspaceError> {
        unimplemented!()
    }

    pub async fn glob(
        &self,
        _user_id: &str,
        _pattern: &str,
        _root: &str,
    ) -> Result<Vec<String>, WorkspaceError> {
        unimplemented!()
    }

    pub async fn grep(
        &self,
        _user_id: &str,
        _pattern: &str,
        _root: &str,
        _opts: GrepOpts,
    ) -> Result<Vec<GrepHit>, WorkspaceError> {
        unimplemented!()
    }

    // ── Quota / admin ─────────────────────────────────────────────────────────

    pub fn quota(&self, _user_id: &str) -> QuotaSnapshot {
        unimplemented!()
    }

    pub async fn wipe_user(&self, _user_id: &str) -> Result<(), WorkspaceError> {
        unimplemented!()
    }
}

#[cfg(test)]
fn new_for_test(root: PathBuf) -> WorkspaceFs {
    WorkspaceFs::new(
        root,
        std::sync::Arc::new(crate::workspace::quota::QuotaCache::new(10 * 1024 * 1024)),
        std::sync::Arc::new(crate::skills_cache::SkillsCache::new()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn read_relative_path_succeeds() {
        let tmp = tempdir().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("hello.txt"), b"hi").await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let bytes = fs.read("alice", "hello.txt").await.unwrap();
        assert_eq!(bytes, b"hi");
    }

    #[tokio::test]
    async fn read_absolute_path_succeeds() {
        let tmp = tempdir().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("hello.txt"), b"hi").await.unwrap();

        let root = tmp.path().to_str().unwrap().to_string();
        let fs = new_for_test(tmp.path().to_path_buf());
        let abs_path = format!("{}/alice/hello.txt", root);
        let bytes = fs.read("alice", &abs_path).await.unwrap();
        assert_eq!(bytes, b"hi");
    }

    #[tokio::test]
    async fn read_dotdot_escape_rejected() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();
        let bob_dir = tmp.path().join("bob");
        tokio::fs::create_dir_all(&bob_dir).await.unwrap();
        tokio::fs::write(bob_dir.join("secret"), b"secrets").await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let result = fs.read("alice", "../bob/secret").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_symlink_escape_rejected() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();
        tokio::fs::symlink("/etc/passwd", alice_dir.join("pw")).await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let result = fs.read("alice", "pw").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }
}
