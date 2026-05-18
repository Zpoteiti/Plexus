use plexus_common::WorkspaceError;
use sqlx::PgPool;
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

#[derive(Clone)]
pub struct WorkspaceFs {
    root: PathBuf,
    pool: PgPool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct QuotaState {
    pub quota_bytes: u64,
    pub bytes_used: u64,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
}

impl WorkspaceFs {
    pub fn new(root: PathBuf, pool: PgPool) -> Self {
        Self { root, pool }
    }

    pub async fn quota(&self, user_id: Uuid) -> Result<QuotaState, WorkspaceError> {
        let quota_bytes = self.quota_bytes().await?;
        let workspace = self.personal_root(user_id);
        fs::create_dir_all(&workspace).await?;
        let bytes_used = dir_size(&workspace).await?;
        Ok(QuotaState {
            quota_bytes,
            bytes_used,
            locked: bytes_used > quota_bytes,
        })
    }

    pub async fn read_file(&self, user_id: Uuid, path: &str) -> Result<Vec<u8>, WorkspaceError> {
        let full = self.resolve_existing(user_id, path).await?;
        let meta = metadata_or_not_found(&full).await?;
        if meta.is_dir() {
            return Err(WorkspaceError::PathOutsideWorkspace(full));
        }
        Ok(fs::read(full).await?)
    }

    pub async fn write_file(
        &self,
        user_id: Uuid,
        path: &str,
        bytes: Vec<u8>,
    ) -> Result<(), WorkspaceError> {
        let full = self.resolve_for_write(user_id, path).await?;
        self.ensure_can_write(user_id, &full, bytes.len() as u64)
            .await?;
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(full, bytes).await?;
        Ok(())
    }

    pub async fn delete_file(&self, user_id: Uuid, path: &str) -> Result<(), WorkspaceError> {
        let full = self.resolve_existing(user_id, path).await?;
        let meta = metadata_or_not_found(&full).await?;
        if meta.is_dir() {
            return Err(WorkspaceError::PathOutsideWorkspace(full));
        }
        fs::remove_file(full).await?;
        Ok(())
    }

    fn personal_root(&self, user_id: Uuid) -> PathBuf {
        self.root.join(user_id.to_string())
    }

    async fn quota_bytes(&self) -> Result<u64, WorkspaceError> {
        let value: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT value FROM system_config WHERE key = 'quota_bytes'")
                .fetch_optional(&self.pool)
                .await
                .map_err(|err| WorkspaceError::IoError(std::io::Error::other(err)))?;
        value
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0)
            .map(|v| v as u64)
            .ok_or(WorkspaceError::QuotaNotConfigured)
    }

    async fn ensure_can_write(
        &self,
        user_id: Uuid,
        target: &Path,
        new_bytes: u64,
    ) -> Result<(), WorkspaceError> {
        let quota = self.quota(user_id).await?;
        if quota.locked {
            return Err(WorkspaceError::SoftLocked);
        }
        if new_bytes > quota.quota_bytes.saturating_mul(80) / 100 {
            return Err(WorkspaceError::UploadTooLarge {
                actual_bytes: new_bytes,
                quota_bytes: quota.quota_bytes,
            });
        }
        let existing_bytes = existing_file_size(target).await?;
        if quota
            .bytes_used
            .saturating_sub(existing_bytes)
            .saturating_add(new_bytes)
            > quota.quota_bytes
        {
            return Err(WorkspaceError::SoftLocked);
        }
        Ok(())
    }

    async fn resolve_existing(&self, user_id: Uuid, path: &str) -> Result<PathBuf, WorkspaceError> {
        let root = self.personal_root(user_id);
        fs::create_dir_all(&root).await?;
        plexus_common::tools::path::resolve_in_workspace(&root, path)
    }

    async fn resolve_for_write(
        &self,
        user_id: Uuid,
        path: &str,
    ) -> Result<PathBuf, WorkspaceError> {
        let requested = Path::new(path);
        if path.is_empty()
            || requested
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(WorkspaceError::PathOutsideWorkspace(PathBuf::from(path)));
        }

        let root = self.personal_root(user_id);
        fs::create_dir_all(&root).await?;
        let canonical_root = root
            .canonicalize()
            .map_err(|_| WorkspaceError::NotFound(root.clone()))?;
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            canonical_root.join(requested)
        };

        for ancestor in candidate.ancestors() {
            let canonical_ancestor = match ancestor.canonicalize() {
                Ok(path) => path,
                Err(_) => continue,
            };
            if !canonical_ancestor.starts_with(&canonical_root) {
                return Err(WorkspaceError::PathOutsideWorkspace(candidate));
            }
            let suffix = candidate
                .strip_prefix(ancestor)
                .map_err(|_| WorkspaceError::PathOutsideWorkspace(candidate.clone()))?;
            return Ok(canonical_ancestor.join(suffix));
        }

        Err(WorkspaceError::PathOutsideWorkspace(candidate))
    }
}

async fn metadata_or_not_found(path: &Path) -> Result<std::fs::Metadata, WorkspaceError> {
    fs::metadata(path).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            WorkspaceError::NotFound(path.to_path_buf())
        } else {
            WorkspaceError::IoError(err)
        }
    })
}

async fn existing_file_size(path: &Path) -> Result<u64, WorkspaceError> {
    match fs::symlink_metadata(path).await {
        Ok(meta) if meta.is_dir() => Err(WorkspaceError::PathOutsideWorkspace(path.to_path_buf())),
        Ok(meta) if meta.is_file() => Ok(meta.len()),
        Ok(_) => Ok(0),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(WorkspaceError::IoError(err)),
    }
}

async fn dir_size(root: &Path) -> Result<u64, WorkspaceError> {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let meta = fs::symlink_metadata(entry.path()).await?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Ok(total)
}
