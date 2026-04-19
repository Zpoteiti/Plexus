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

    /// Resolve `path` (absolute or relative) to a `PathBuf` guaranteed to be
    /// inside `<root>/<user_id>/`. Unlike `resolve_path`, the tail components
    /// need not exist yet (for writes/creates). Returns
    /// `WorkspaceError::Traversal` on any escape attempt, with a `warn!` log.
    async fn resolve_path_for_create(
        &self,
        user_id: &str,
        path: &str,
    ) -> Result<PathBuf, WorkspaceError> {
        let result = if std::path::Path::new(path).is_absolute() {
            // Walk up to the nearest existing ancestor, canonicalize it, prefix-check,
            // then re-attach the non-existent tail components.
            let mut ancestor = std::path::PathBuf::from(path);
            let mut tail: Vec<std::ffi::OsString> = Vec::new();
            while tokio::fs::symlink_metadata(&ancestor).await.is_err() {
                let component = ancestor
                    .file_name()
                    .ok_or_else(|| WorkspaceError::Traversal(path.into()))?
                    .to_owned();
                tail.push(component);
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| WorkspaceError::Traversal(path.into()))?
                    .to_path_buf();
            }
            let canonical_ancestor = tokio::fs::canonicalize(&ancestor).await?;
            let user_root_canonical =
                tokio::fs::canonicalize(self.root.join(user_id)).await?;
            if !canonical_ancestor.starts_with(&user_root_canonical) {
                return Err(WorkspaceError::Traversal(path.into()));
            }
            let mut result = canonical_ancestor;
            for component in tail.into_iter().rev() {
                if component == std::ffi::OsStr::new("..") || component == std::ffi::OsStr::new(".") {
                    return Err(WorkspaceError::Traversal(path.into()));
                }
                result.push(component);
            }
            Ok(result)
        } else {
            crate::workspace::paths::resolve_user_path_for_create(&self.root, user_id, path)
                .await
                .map_err(|e| match e {
                    crate::workspace::paths::WorkspaceError::Traversal(s) => {
                        WorkspaceError::Traversal(s)
                    }
                    crate::workspace::paths::WorkspaceError::Io(io) => WorkspaceError::Io(io),
                    crate::workspace::paths::WorkspaceError::Quota(_) => {
                        WorkspaceError::Traversal(path.into())
                    }
                })
        };
        if let Err(WorkspaceError::Traversal(_)) = &result {
            warn!(user_id, path, "workspace path escape attempt");
        }
        result
    }

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
        user_id: &str,
        path: &str,
        bytes: &[u8],
    ) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path_for_create(user_id, path).await?;

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        self.quota
            .check_and_reserve_upload(user_id, bytes.len() as u64)
            .map_err(|e| match e {
                crate::workspace::quota::QuotaError::UploadTooLarge(actual, limit) => {
                    WorkspaceError::UploadTooLarge { actual, limit }
                }
                crate::workspace::quota::QuotaError::SoftLocked(_, _) => {
                    WorkspaceError::SoftLocked
                }
            })?;

        if let Err(io_err) = tokio::fs::write(&resolved, bytes).await {
            self.quota.release(user_id, bytes.len() as u64);
            return Err(WorkspaceError::Io(io_err));
        }

        if crate::workspace::paths::is_under_skills_dir(&resolved, &self.root, user_id) {
            self.skills_cache.invalidate(user_id);
        }

        Ok(())
    }

    pub async fn write_stream<R: tokio::io::AsyncRead + Unpin>(
        &self,
        user_id: &str,
        path: &str,
        mut reader: R,
        expected_size: u64,
    ) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path_for_create(user_id, path).await?;

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        self.quota
            .check_and_reserve_upload(user_id, expected_size)
            .map_err(|e| match e {
                crate::workspace::quota::QuotaError::UploadTooLarge(actual, limit) => {
                    WorkspaceError::UploadTooLarge { actual, limit }
                }
                crate::workspace::quota::QuotaError::SoftLocked(_, _) => {
                    WorkspaceError::SoftLocked
                }
            })?;

        let mut file = match tokio::fs::File::create(&resolved).await {
            Ok(f) => f,
            Err(io_err) => {
                self.quota.release(user_id, expected_size);
                return Err(WorkspaceError::Io(io_err));
            }
        };

        let copy_result = tokio::io::copy(&mut reader, &mut file).await;
        match copy_result {
            Ok(written) if written == expected_size => {
                // Success path — check skills invalidation.
                if crate::workspace::paths::is_under_skills_dir(&resolved, &self.root, user_id) {
                    self.skills_cache.invalidate(user_id);
                }
                Ok(())
            }
            Ok(written) => {
                // Mismatch between actual and expected bytes.
                self.quota.release(user_id, expected_size);
                let _ = tokio::fs::remove_file(&resolved).await;
                Err(WorkspaceError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("expected {expected_size} bytes but wrote {written}"),
                )))
            }
            Err(io_err) => {
                self.quota.release(user_id, expected_size);
                let _ = tokio::fs::remove_file(&resolved).await;
                Err(WorkspaceError::Io(io_err))
            }
        }
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

    pub fn quota(&self, user_id: &str) -> QuotaSnapshot {
        QuotaSnapshot {
            used_bytes: self.quota.current_usage(user_id),
            limit_bytes: self.quota.quota_bytes(),
        }
    }

    pub async fn wipe_user(&self, _user_id: &str) -> Result<(), WorkspaceError> {
        unimplemented!()
    }
}

#[cfg(test)]
fn new_for_test(root: PathBuf) -> WorkspaceFs {
    new_for_test_with_quota(root, 10 * 1024 * 1024)
}

#[cfg(test)]
fn new_for_test_with_quota(root: PathBuf, quota_bytes: u64) -> WorkspaceFs {
    WorkspaceFs::new(
        root,
        std::sync::Arc::new(crate::workspace::quota::QuotaCache::new(quota_bytes)),
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

    // ── P3.3 write tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn write_within_quota_succeeds_and_reserves() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        let fs = new_for_test_with_quota(tmp.path().to_path_buf(), 1024 * 1024);
        fs.write("alice", "a.txt", b"hello world").await.unwrap();

        // File on disk with expected content.
        let on_disk = tokio::fs::read(alice_dir.join("a.txt")).await.unwrap();
        assert_eq!(on_disk, b"hello world");

        // Quota counter reflects the write.
        let snap = fs.quota("alice");
        assert_eq!(snap.used_bytes, 11);
    }

    #[tokio::test]
    async fn write_exceeding_per_upload_cap_rejected_no_file_written() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        // 1 MB quota → 800 KB per-upload cap; attempt 900 KB write.
        let fs = new_for_test_with_quota(tmp.path().to_path_buf(), 1024 * 1024);
        let big = vec![0u8; 900_000];
        let result = fs.write("alice", "big.bin", &big).await;
        assert!(matches!(result, Err(WorkspaceError::UploadTooLarge { .. })));

        // No file on disk.
        assert!(!alice_dir.join("big.bin").exists());

        // No quota reserved.
        let snap = fs.quota("alice");
        assert_eq!(snap.used_bytes, 0);
    }

    #[tokio::test]
    async fn write_to_skills_subdir_invalidates_cache() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());

        // Prime the cache with an initial (empty) load.
        let before = fs.skills_cache.get_or_load("alice", &fs.root).await;

        // Write into the skills directory.
        fs.write(
            "alice",
            "skills/git/SKILL.md",
            b"---\nname: git\ndescription: git tool\nalways_on: false\n---\nbody",
        )
        .await
        .unwrap();

        // After write, cache should have been invalidated; a fresh load gives a
        // new Arc (different pointer).
        let after = fs.skills_cache.get_or_load("alice", &fs.root).await;
        assert!(
            !std::sync::Arc::ptr_eq(&before, &after),
            "expected cache to be invalidated and reloaded after skills write"
        );
    }

    #[tokio::test]
    async fn attachments_count_against_quota() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        fs.write("alice", ".attachments/msg-1/img.png", b"12345678")
            .await
            .unwrap();

        let snap = fs.quota("alice");
        assert_eq!(snap.used_bytes, 8);
    }
}
