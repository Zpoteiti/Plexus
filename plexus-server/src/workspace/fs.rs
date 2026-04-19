//! Workspace filesystem service.
//!
//! `WorkspaceFs` is the single entry point for all per-user file I/O.
//! It enforces path traversal checks, quota, and skills-cache invalidation.
//! Method bodies are stubbed — implementations land in P3.2–P3.5.

use std::path::PathBuf;
use std::sync::Arc;

use plexus_common::errors::workspace::WorkspaceError;

// ── Supporting types ──────────────────────────────────────────────────────────

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

    // ── Reads ─────────────────────────────────────────────────────────────────

    pub async fn read(&self, _user_id: &str, _path: &str) -> Result<Vec<u8>, WorkspaceError> {
        unimplemented!()
    }

    pub async fn read_stream(
        &self,
        _user_id: &str,
        _path: &str,
    ) -> Result<tokio_util::io::ReaderStream<tokio::fs::File>, WorkspaceError> {
        unimplemented!()
    }

    pub async fn stat(&self, _user_id: &str, _path: &str) -> Result<FileStat, WorkspaceError> {
        unimplemented!()
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
