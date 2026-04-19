//! Workspace filesystem service.
//!
//! `WorkspaceFs` is the single entry point for all per-user file I/O.
//! It enforces path traversal checks, quota, and skills-cache invalidation.
//! Method bodies are stubbed — implementations land in P3.2–P3.5.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use plexus_common::errors::workspace::WorkspaceError;
use tracing::{debug, warn};

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

#[derive(Debug, PartialEq, Eq)]
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

#[derive(Debug, PartialEq, Eq)]
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
            let user_root_canonical = tokio::fs::canonicalize(self.root.join(user_id)).await?;
            if !canonical_ancestor.starts_with(&user_root_canonical) {
                return Err(WorkspaceError::Traversal(path.into()));
            }
            let mut result = canonical_ancestor;
            for component in tail.into_iter().rev() {
                if component == std::ffi::OsStr::new("..") || component == std::ffi::OsStr::new(".")
                {
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
            let user_root_canonical = tokio::fs::canonicalize(self.root.join(user_id)).await?;
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
        let mime = plexus_common::mime::detect_mime_from_extension(resolved.to_str().unwrap_or(""))
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

        // Delta-aware quota: only reserve the *net increase* in bytes.
        let existing = tokio::fs::metadata(&resolved)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let new_size = bytes.len() as u64;
        let delta = new_size.saturating_sub(existing);

        if delta > 0 {
            self.quota
                .check_and_reserve_upload(user_id, delta)
                .map_err(|e| match e {
                    crate::workspace::quota::QuotaError::UploadTooLarge(actual, limit) => {
                        WorkspaceError::UploadTooLarge { actual, limit }
                    }
                    crate::workspace::quota::QuotaError::SoftLocked(_, _) => {
                        WorkspaceError::SoftLocked
                    }
                })?;
        }

        if let Err(io_err) = tokio::fs::write(&resolved, bytes).await {
            if delta > 0 {
                self.quota.release(user_id, delta);
            }
            return Err(WorkspaceError::Io(io_err));
        }

        // If we shrank the file, release the reclaimed bytes from quota.
        if new_size < existing {
            self.quota.record_delete(user_id, existing - new_size);
        }

        if crate::workspace::paths::is_under_skills_dir(&resolved, &self.root, user_id) {
            self.skills_cache.invalidate(user_id);
        }

        Ok(())
    }

    // TODO: make delta-aware before any overwrite-capable caller wires in; today
    // only `write` handles shrink/grow quota math correctly.
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
                crate::workspace::quota::QuotaError::SoftLocked(_, _) => WorkspaceError::SoftLocked,
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

    pub async fn delete(&self, user_id: &str, path: &str) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path(user_id, path).await?;

        // Fetch size before deletion so quota can be decremented accurately.
        let meta = tokio::fs::metadata(&resolved).await?;
        let bytes_freed = meta.len();

        tokio::fs::remove_file(&resolved).await?;

        self.quota.record_delete(user_id, bytes_freed);

        if crate::workspace::paths::is_under_skills_dir(&resolved, &self.root, user_id) {
            self.skills_cache.invalidate(user_id);
        }

        Ok(())
    }

    pub async fn delete_prefix(
        &self,
        user_id: &str,
        prefix: &str,
        older_than: Option<std::time::Duration>,
    ) -> Result<u64, WorkspaceError> {
        // Resolve prefix — if it doesn't exist, treat as no-op.
        let resolved = match self.resolve_path(user_id, prefix).await {
            Ok(p) => p,
            Err(WorkspaceError::Io(_)) => return Ok(0),
            Err(e) => return Err(e),
        };

        // If the resolved path isn't a directory, no-op.
        let meta = tokio::fs::metadata(&resolved).await;
        match meta {
            Ok(m) if m.is_dir() => {}
            _ => return Ok(0),
        }

        let invalidate_skills =
            crate::workspace::paths::is_under_skills_dir(&resolved, &self.root, user_id)
                || resolved == self.root.join(user_id).join("skills");

        let mut total_reclaimed: u64 = 0;

        // Async directory walk via an explicit stack (avoids sync walkdir).
        let mut dir_stack = vec![resolved.clone()];
        while let Some(dir) = dir_stack.pop() {
            let mut read_dir = match tokio::fs::read_dir(&dir).await {
                Ok(rd) => rd,
                Err(_) => continue,
            };

            while let Some(entry) = read_dir.next_entry().await? {
                let file_type = entry.file_type().await?;
                if file_type.is_dir() {
                    dir_stack.push(entry.path());
                } else if file_type.is_file() {
                    let entry_meta = entry.metadata().await?;
                    // Apply TTL filter if requested.
                    if let Some(dur) = older_than {
                        let mtime = entry_meta.modified()?;
                        let age = SystemTime::now().duration_since(mtime).unwrap_or_default();
                        if age < dur {
                            continue; // file is too recent — skip
                        }
                    }
                    let size = entry_meta.len();
                    if let Err(e) = tokio::fs::remove_file(entry.path()).await {
                        debug!("delete_prefix: failed to remove {:?}: {e}", entry.path());
                    } else {
                        total_reclaimed += size;
                    }
                }
            }
        }

        // Remove empty directories left behind under the prefix.
        // Walk depth-first: collect all subdirs, sort deepest first, remove if empty.
        let mut dirs_to_clean: Vec<PathBuf> = Vec::new();
        let mut scan_stack = vec![resolved.clone()];
        while let Some(dir) = scan_stack.pop() {
            let mut rd = match tokio::fs::read_dir(&dir).await {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            while let Some(entry) = rd.next_entry().await? {
                if entry.file_type().await?.is_dir() {
                    scan_stack.push(entry.path());
                    dirs_to_clean.push(entry.path());
                }
            }
        }
        // Sort longest path first so we remove leaf dirs before parents.
        dirs_to_clean.sort_by(|a, b| b.components().count().cmp(&a.components().count()));
        for dir in dirs_to_clean {
            // Ignore errors — directory may not be empty or already removed.
            let _ = tokio::fs::remove_dir(&dir).await;
        }

        if total_reclaimed > 0 {
            self.quota.record_delete(user_id, total_reclaimed);
        }

        if invalidate_skills {
            self.skills_cache.invalidate(user_id);
        }

        Ok(total_reclaimed)
    }

    /// Delete a file or directory at `path`.
    ///
    /// - File: delegates to the same logic as `delete`.
    /// - Directory + `recursive=true`: removes all contents, frees quota, then
    ///   removes the directory tree.
    /// - Directory + `recursive=false`: returns an `Io` error.
    pub async fn delete_path(
        &self,
        user_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path(user_id, path).await?;
        let meta = tokio::fs::metadata(&resolved).await?;

        if meta.is_file() {
            // Delegate to the single-file deletion path.
            let bytes_freed = meta.len();
            tokio::fs::remove_file(&resolved).await?;
            self.quota.record_delete(user_id, bytes_freed);
            if crate::workspace::paths::is_under_skills_dir(&resolved, &self.root, user_id) {
                self.skills_cache.invalidate(user_id);
            }
        } else if meta.is_dir() {
            if !recursive {
                return Err(WorkspaceError::Io(std::io::Error::other(
                    "directory requires recursive",
                )));
            }
            // Walk to sum file sizes before removal, for accurate quota accounting.
            let freed = crate::workspace::quota::walk_dir_bytes(&resolved)
                .await
                .unwrap_or(0);
            tokio::fs::remove_dir_all(&resolved).await?;
            if freed > 0 {
                self.quota.record_delete(user_id, freed);
            }
            // Invalidate skills cache if we deleted under the skills dir.
            if crate::workspace::paths::is_under_skills_dir(&resolved, &self.root, user_id)
                || resolved == self.root.join(user_id).join("skills")
            {
                self.skills_cache.invalidate(user_id);
            }
        }

        Ok(())
    }

    // ── Directory / search ────────────────────────────────────────────────────

    pub async fn list(&self, user_id: &str, path: &str) -> Result<Vec<DirEntry>, WorkspaceError> {
        let resolved = self.resolve_path(user_id, path).await?;

        let meta = tokio::fs::metadata(&resolved).await?;
        if !meta.is_dir() {
            return Err(WorkspaceError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{path} is not a directory"),
            )));
        }

        let mut read_dir = tokio::fs::read_dir(&resolved).await?;
        let mut entries = Vec::new();

        while let Some(entry) = read_dir.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // If metadata fails (stale symlink, race), skip the entry and log at
            // debug level — one bad entry should not abort the entire listing.
            let entry_meta = match entry.metadata().await {
                Ok(m) => m,
                Err(e) => {
                    debug!("list: skipping entry {name:?}: {e}");
                    continue;
                }
            };
            let kind = if entry_meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            let size = if entry_meta.is_dir() {
                0
            } else {
                entry_meta.len()
            };
            entries.push(DirEntry { name, kind, size });
        }

        Ok(entries)
    }

    pub async fn glob(
        &self,
        user_id: &str,
        pattern: &str,
        root: &str,
    ) -> Result<Vec<String>, WorkspaceError> {
        let user_root = tokio::fs::canonicalize(self.root.join(user_id))
            .await
            .map_err(WorkspaceError::Io)?;

        let search_root = if root.is_empty() {
            user_root.clone()
        } else {
            self.resolve_path(user_id, root).await?
        };

        let matcher = globset::Glob::new(pattern)
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("bad glob pattern: {e}"),
                ))
            })?
            .compile_matcher();

        let user_root_clone = user_root.clone();
        let matches = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for entry in walkdir::WalkDir::new(&search_root)
                .into_iter()
                .filter_map(Result::ok)
            {
                if entry.path() == search_root {
                    continue;
                }
                let rel = match entry.path().strip_prefix(&user_root_clone) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                if matcher.is_match(rel) {
                    out.push(rel.display().to_string());
                }
                if out.len() >= 500 {
                    break;
                }
            }
            out
        })
        .await
        .map_err(|e| {
            WorkspaceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("spawn_blocking panic: {e}"),
            ))
        })?;

        Ok(matches)
    }

    pub async fn grep(
        &self,
        user_id: &str,
        pattern: &str,
        root: &str,
        opts: GrepOpts,
    ) -> Result<Vec<GrepHit>, WorkspaceError> {
        let user_root = tokio::fs::canonicalize(self.root.join(user_id))
            .await
            .map_err(WorkspaceError::Io)?;

        let search_root = if root.is_empty() {
            user_root.clone()
        } else {
            self.resolve_path(user_id, root).await?
        };

        let regex_pattern = if opts.case_insensitive {
            format!("(?i){pattern}")
        } else {
            pattern.to_string()
        };

        let re = regex::Regex::new(&regex_pattern).map_err(|e| {
            WorkspaceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("bad regex pattern: {e}"),
            ))
        })?;

        let user_root_clone = user_root.clone();
        let hits = tokio::task::spawn_blocking(move || {
            let mut out: Vec<GrepHit> = Vec::new();
            for entry in walkdir::WalkDir::new(&search_root)
                .into_iter()
                .filter_map(Result::ok)
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let content = match std::fs::read_to_string(entry.path()) {
                    Ok(c) => c,
                    Err(_) => continue, // skip non-UTF-8 files
                };
                let rel = match entry.path().strip_prefix(&user_root_clone) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        out.push(GrepHit {
                            path: rel.display().to_string(),
                            line_number: (i + 1) as u64,
                            line_content: line.to_string(),
                        });
                        if out.len() >= 200 {
                            return out;
                        }
                    }
                }
            }
            out
        })
        .await
        .map_err(|e| {
            WorkspaceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("spawn_blocking panic: {e}"),
            ))
        })?;

        Ok(hits)
    }

    // ── Quota / admin ─────────────────────────────────────────────────────────

    pub fn quota(&self, user_id: &str) -> QuotaSnapshot {
        QuotaSnapshot {
            used_bytes: self.quota.current_usage(user_id),
            limit_bytes: self.quota.quota_bytes(),
        }
    }

    pub async fn wipe_user(&self, user_id: &str) -> Result<(), WorkspaceError> {
        let user_dir = self.root.join(user_id);
        match tokio::fs::remove_dir_all(&user_dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Already wiped — treat as success (idempotent).
            }
            Err(e) => return Err(WorkspaceError::Io(e)),
        }
        self.quota.forget_user(user_id);
        self.skills_cache.invalidate(user_id);
        Ok(())
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
    #[cfg(unix)]
    use filetime;
    use tempfile::tempdir;

    #[tokio::test]
    async fn read_relative_path_succeeds() {
        let tmp = tempdir().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("hello.txt"), b"hi")
            .await
            .unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let bytes = fs.read("alice", "hello.txt").await.unwrap();
        assert_eq!(bytes, b"hi");
    }

    #[tokio::test]
    async fn read_absolute_path_succeeds() {
        let tmp = tempdir().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("hello.txt"), b"hi")
            .await
            .unwrap();

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
        tokio::fs::write(bob_dir.join("secret"), b"secrets")
            .await
            .unwrap();

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
        tokio::fs::symlink("/etc/passwd", alice_dir.join("pw"))
            .await
            .unwrap();

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

    // ── P3.4 delete/delete_prefix/list/wipe_user tests ───────────────────────

    #[tokio::test]
    async fn delete_single_file_decrements_quota() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        fs.write("alice", "a.txt", &[0u8; 100]).await.unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 100);

        fs.delete("alice", "a.txt").await.unwrap();

        assert_eq!(fs.quota("alice").used_bytes, 0);
        assert!(!alice_dir.join("a.txt").exists());
    }

    #[tokio::test]
    async fn delete_skills_file_invalidates_cache() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());

        // Write a skills file first so cache has something to load.
        fs.write(
            "alice",
            "skills/git/SKILL.md",
            b"---\nname: git\ndescription: git tool\nalways_on: false\n---\nbody",
        )
        .await
        .unwrap();

        // Prime the cache.
        let before = fs.skills_cache.get_or_load("alice", &fs.root).await;

        // Delete the skills file — should invalidate cache.
        fs.delete("alice", "skills/git/SKILL.md").await.unwrap();

        // Fresh load must yield a different Arc.
        let after = fs.skills_cache.get_or_load("alice", &fs.root).await;
        assert!(
            !std::sync::Arc::ptr_eq(&before, &after),
            "expected cache invalidation after skills file deletion"
        );
        drop(alice_dir);
    }

    #[tokio::test]
    async fn delete_prefix_with_ttl_reclaims_old_files() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());

        // Write two files.
        fs.write("alice", ".attachments/old/a.bin", &[1u8; 50])
            .await
            .unwrap();
        fs.write("alice", ".attachments/new/b.bin", &[2u8; 30])
            .await
            .unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 80);

        // Set mtime of old/a.bin to 60 days ago.
        let old_path = alice_dir.join(".attachments/old/a.bin");
        let sixty_days_ago = filetime::FileTime::from_unix_time(
            (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64)
                - 60 * 86400,
            0,
        );
        filetime::set_file_mtime(&old_path, sixty_days_ago).unwrap();

        // Delete files older than 30 days.
        let reclaimed = fs
            .delete_prefix(
                "alice",
                ".attachments",
                Some(std::time::Duration::from_secs(30 * 86400)),
            )
            .await
            .unwrap();

        assert_eq!(reclaimed, 50, "should reclaim only the old file");
        assert!(!old_path.exists(), "old file must be gone");
        assert!(
            alice_dir.join(".attachments/new/b.bin").exists(),
            "new file must remain"
        );
        assert_eq!(fs.quota("alice").used_bytes, 30);
    }

    #[tokio::test]
    async fn list_returns_top_level_entries() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();
        tokio::fs::write(alice_dir.join("a.txt"), b"hello")
            .await
            .unwrap();
        tokio::fs::create_dir_all(alice_dir.join("sub"))
            .await
            .unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let mut entries = fs.list("alice", ".").await.unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[0].kind, EntryKind::File);
        assert_eq!(entries[1].name, "sub");
        assert_eq!(entries[1].kind, EntryKind::Dir);
    }

    // ── P3.5 glob + grep tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn glob_matches_extension_pattern() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(alice_dir.join("nested"))
            .await
            .unwrap();
        tokio::fs::write(alice_dir.join("a.rs"), b"fn a() {}")
            .await
            .unwrap();
        tokio::fs::write(alice_dir.join("b.rs"), b"fn b() {}")
            .await
            .unwrap();
        tokio::fs::write(alice_dir.join("c.py"), b"def c(): pass")
            .await
            .unwrap();
        tokio::fs::write(alice_dir.join("nested/d.rs"), b"fn d() {}")
            .await
            .unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let results = fs.glob("alice", "**/*.rs", "").await.unwrap();

        assert_eq!(
            results.len(),
            3,
            "expected exactly 3 .rs files, got: {results:?}"
        );
        let set: std::collections::HashSet<&str> = results.iter().map(|s| s.as_str()).collect();
        assert!(
            set.iter().all(|p| p.ends_with(".rs")),
            "all paths must end in .rs"
        );
    }

    #[tokio::test]
    async fn glob_with_subdir_root_returns_relative_paths() {
        let tmp = tempdir().unwrap();
        let src_dir = tmp.path().join("alice/project/src");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        tokio::fs::write(src_dir.join("lib.rs"), b"pub fn lib() {}")
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("main.rs"), b"fn main() {}")
            .await
            .unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let results = fs.glob("alice", "**/*.rs", "project/src").await.unwrap();

        assert_eq!(
            results.len(),
            2,
            "expected exactly 2 .rs files, got: {results:?}"
        );
        for path in &results {
            assert!(
                !path.starts_with('/'),
                "result must not be an absolute path: {path:?}"
            );
        }
    }

    #[tokio::test]
    async fn grep_finds_pattern_line_numbered() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();
        tokio::fs::write(
            alice_dir.join("doc.txt"),
            b"one\ntwo three\nTHREE\nfour three",
        )
        .await
        .unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        let hits = fs
            .grep("alice", "three", "", GrepOpts::default())
            .await
            .unwrap();

        assert_eq!(
            hits.len(),
            2,
            "expected 2 case-sensitive hits, got: {hits:?}"
        );
        let line_numbers: Vec<u64> = hits.iter().map(|h| h.line_number).collect();
        assert!(line_numbers.contains(&2), "line 2 must be a hit");
        assert!(line_numbers.contains(&4), "line 4 must be a hit");
        for hit in &hits {
            assert!(
                hit.line_content.contains("three"),
                "each hit must contain 'three': {:?}",
                hit.line_content
            );
        }
    }

    // ── P3.7 delta-aware write + delete_path tests ───────────────────────────

    #[tokio::test]
    async fn write_overwrite_adjusts_quota_by_delta() {
        let dir = tempdir().unwrap();
        let fs = new_for_test(dir.path().to_path_buf());
        tokio::fs::create_dir_all(dir.path().join("alice"))
            .await
            .unwrap();

        fs.write("alice", "notes.md", &[0u8; 100]).await.unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 100);

        // Shrink: overwrite with 40 bytes → quota should drop to 40.
        fs.write("alice", "notes.md", &[0u8; 40]).await.unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 40);

        // Grow: overwrite with 120 bytes → quota should be 120.
        fs.write("alice", "notes.md", &[0u8; 120]).await.unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 120);
    }

    #[tokio::test]
    async fn delete_path_removes_directory_recursively_and_frees_quota() {
        let dir = tempdir().unwrap();
        let fs = new_for_test(dir.path().to_path_buf());
        tokio::fs::create_dir_all(dir.path().join("alice/project/sub"))
            .await
            .unwrap();
        fs.write("alice", "project/a.txt", &[0u8; 10])
            .await
            .unwrap();
        fs.write("alice", "project/sub/b.txt", &[0u8; 20])
            .await
            .unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 30);

        fs.delete_path("alice", "project", true).await.unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 0);
        assert!(!dir.path().join("alice/project").exists());
    }

    #[tokio::test]
    async fn delete_path_refuses_directory_without_recursive_flag() {
        let dir = tempdir().unwrap();
        let fs = new_for_test(dir.path().to_path_buf());
        tokio::fs::create_dir_all(dir.path().join("alice/subdir"))
            .await
            .unwrap();

        let r = fs.delete_path("alice", "subdir", false).await;
        assert!(r.is_err());
        assert!(dir.path().join("alice/subdir").exists());
    }

    #[tokio::test]
    async fn wipe_user_clears_tree_and_quota() {
        let tmp = tempdir().unwrap();
        let alice_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&alice_dir).await.unwrap();

        let fs = new_for_test(tmp.path().to_path_buf());
        fs.write("alice", "file1.txt", &[0u8; 500]).await.unwrap();
        assert_eq!(fs.quota("alice").used_bytes, 500);

        fs.wipe_user("alice").await.unwrap();

        assert!(!alice_dir.exists(), "alice dir must be removed");
        assert_eq!(
            fs.quota("alice").used_bytes,
            0,
            "quota must be reset after wipe"
        );

        // Calling again should be idempotent (no error).
        fs.wipe_user("alice").await.unwrap();
    }
}
