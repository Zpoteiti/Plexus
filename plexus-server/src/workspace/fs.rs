use plexus_common::WorkspaceError;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    io::{Error, ErrorKind},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::fs;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct WorkspaceFs {
    root: PathBuf,
    pool: PgPool,
    user_locks: Arc<Mutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>>,
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
        Self {
            root,
            pool,
            user_locks: Arc::new(Mutex::new(HashMap::new())),
        }
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
        let meta = final_target_metadata_or_not_found(path, &self.personal_root(user_id)).await?;
        if !meta.is_file() {
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
        let user_lock = self.user_lock(user_id);
        let _guard = user_lock.lock().await;
        let full = self.resolve_for_write(user_id, path).await?;
        self.ensure_can_write(user_id, path, bytes.len() as u64)
            .await?;
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(full, bytes).await?;
        Ok(())
    }

    pub async fn delete_file(&self, user_id: Uuid, path: &str) -> Result<(), WorkspaceError> {
        let user_lock = self.user_lock(user_id);
        let _guard = user_lock.lock().await;
        let full = self.resolve_existing(user_id, path).await?;
        let meta = final_target_metadata_or_not_found(path, &self.personal_root(user_id)).await?;
        if !meta.is_file() {
            return Err(WorkspaceError::PathOutsideWorkspace(full));
        }
        fs::remove_file(full).await?;
        Ok(())
    }

    pub async fn edit_file(
        &self,
        user_id: Uuid,
        path: &str,
        old_text: &str,
        new_text: &str,
        replace_all: bool,
    ) -> Result<usize, WorkspaceError> {
        let bytes = self.read_file(user_id, path).await?;
        let mut text =
            String::from_utf8(bytes).map_err(|err| Error::new(ErrorKind::InvalidData, err))?;

        let replacements = if replace_all {
            let replacements = text.matches(old_text).count();
            if replacements == 0 {
                return Err(WorkspaceError::NotFound(PathBuf::from(path)));
            }
            text = text.replace(old_text, new_text);
            replacements
        } else {
            let Some(start) = text.find(old_text) else {
                return Err(WorkspaceError::NotFound(PathBuf::from(path)));
            };
            text.replace_range(start..start + old_text.len(), new_text);
            1
        };

        self.write_file(user_id, path, text.into_bytes()).await?;
        Ok(replacements)
    }

    pub async fn delete_folder(&self, user_id: Uuid, path: &str) -> Result<(), WorkspaceError> {
        let user_lock = self.user_lock(user_id);
        let _guard = user_lock.lock().await;
        let full = self.resolve_existing(user_id, path).await?;
        let meta = final_target_metadata_or_not_found(path, &self.personal_root(user_id)).await?;
        if !meta.is_dir() {
            return Err(WorkspaceError::PathOutsideWorkspace(full));
        }
        fs::remove_dir_all(full).await?;
        Ok(())
    }

    pub async fn list_dir(
        &self,
        user_id: Uuid,
        path: &str,
    ) -> Result<Vec<DirEntry>, WorkspaceError> {
        let full = self.resolve_existing(user_id, path).await?;
        let root = self.personal_root(user_id);
        let meta = final_target_metadata_or_not_found(path, &root).await?;
        if !meta.is_dir() {
            return Err(WorkspaceError::PathOutsideWorkspace(full));
        }

        let canonical_root = root
            .canonicalize()
            .map_err(|_| WorkspaceError::NotFound(root.clone()))?;
        let mut entries = Vec::new();
        let mut dir = fs::read_dir(&full).await?;
        while let Some(entry) = dir.next_entry().await? {
            let meta = fs::symlink_metadata(entry.path()).await?;
            let name = entry.file_name().to_string_lossy().to_string();
            let path = relative_workspace_path(&canonical_root, &entry.path())?;
            let kind = if meta.file_type().is_symlink() {
                "symlink"
            } else if meta.is_dir() {
                "directory"
            } else if meta.is_file() {
                "file"
            } else {
                "other"
            };
            entries.push(DirEntry {
                name,
                path,
                kind: kind.to_string(),
                size: if meta.is_file() { meta.len() } else { 0 },
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    pub async fn glob(&self, user_id: Uuid, pattern: &str) -> Result<Vec<String>, WorkspaceError> {
        let glob = globset::Glob::new(pattern)
            .map_err(|err| Error::new(ErrorKind::InvalidInput, err))?
            .compile_matcher();
        let root = self.personal_root(user_id);
        fs::create_dir_all(&root).await?;
        let canonical_root = root
            .canonicalize()
            .map_err(|_| WorkspaceError::NotFound(root.clone()))?;

        let mut matches = Vec::new();
        for file in collect_regular_files(&canonical_root).await? {
            let relative = relative_workspace_path(&canonical_root, &file)?;
            if glob.is_match(&relative) {
                matches.push(relative);
            }
        }
        matches.sort();
        Ok(matches)
    }

    pub async fn grep(
        &self,
        user_id: Uuid,
        pattern: &str,
        path: Option<&str>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let regex =
            regex::Regex::new(pattern).map_err(|err| Error::new(ErrorKind::InvalidInput, err))?;
        let root = self.personal_root(user_id);
        fs::create_dir_all(&root).await?;
        let canonical_root = root
            .canonicalize()
            .map_err(|_| WorkspaceError::NotFound(root.clone()))?;

        let files = if let Some(path) = path {
            let full = self.resolve_existing(user_id, path).await?;
            let meta = final_target_metadata_or_not_found(path, &root).await?;
            if meta.is_file() {
                vec![full]
            } else if meta.is_dir() {
                collect_regular_files(&full).await?
            } else {
                Vec::new()
            }
        } else {
            collect_regular_files(&canonical_root).await?
        };

        let mut matches = Vec::new();
        for file in files {
            grep_file(&canonical_root, &file, &regex, &mut matches).await?;
        }
        matches.sort();
        Ok(matches)
    }

    fn personal_root(&self, user_id: Uuid) -> PathBuf {
        self.root.join(user_id.to_string())
    }

    fn user_lock(&self, user_id: Uuid) -> Arc<AsyncMutex<()>> {
        let mut locks = self.user_locks.lock().unwrap();
        locks
            .entry(user_id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
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
        path: &str,
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
        let existing_bytes = existing_file_size(path, &self.personal_root(user_id)).await?;
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
            if suffix.as_os_str().is_empty() {
                return Ok(canonical_ancestor);
            }
            return Ok(canonical_ancestor.join(suffix));
        }

        Err(WorkspaceError::PathOutsideWorkspace(candidate))
    }
}

async fn final_target_metadata_or_not_found(
    path: &str,
    root: &Path,
) -> Result<std::fs::Metadata, WorkspaceError> {
    let requested = requested_path(path, root).await?;
    fs::symlink_metadata(&requested).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            WorkspaceError::NotFound(requested)
        } else {
            WorkspaceError::IoError(err)
        }
    })
}

async fn existing_file_size(path: &str, root: &Path) -> Result<u64, WorkspaceError> {
    let requested = requested_path(path, root).await?;
    match fs::symlink_metadata(&requested).await {
        Ok(meta) if meta.is_dir() => Err(WorkspaceError::PathOutsideWorkspace(requested)),
        Ok(meta) if meta.is_file() => Ok(meta.len()),
        Ok(_) => Err(WorkspaceError::PathOutsideWorkspace(requested)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(WorkspaceError::IoError(err)),
    }
}

async fn requested_path(path: &str, root: &Path) -> Result<PathBuf, WorkspaceError> {
    let requested = Path::new(path);
    if requested.is_absolute() {
        Ok(requested.to_path_buf())
    } else {
        fs::create_dir_all(root).await?;
        Ok(root.join(requested))
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

async fn collect_regular_files(root: &Path) -> Result<Vec<PathBuf>, WorkspaceError> {
    let mut files = Vec::new();
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
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

async fn grep_file(
    root: &Path,
    file: &Path,
    regex: &regex::Regex,
    matches: &mut Vec<String>,
) -> Result<(), WorkspaceError> {
    let Ok(contents) = fs::read_to_string(file).await else {
        return Ok(());
    };
    let relative = relative_workspace_path(root, file)?;
    for (line_index, line) in contents.lines().enumerate() {
        if regex.is_match(line) {
            matches.push(format!("{}:{}:{}", relative, line_index + 1, line));
        }
    }
    Ok(())
}

fn relative_workspace_path(root: &Path, path: &Path) -> Result<String, WorkspaceError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| WorkspaceError::PathOutsideWorkspace(path.to_path_buf()))?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}
