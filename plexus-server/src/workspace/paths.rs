use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("path traversal attempt: {0}")]
    Traversal(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve a relative user-workspace path against `{workspace_root}/{user_id}/`.
/// Canonicalizes (resolves symlinks) and rejects paths escaping the user root.
/// The target must exist; use `resolve_user_path_for_create` for paths that don't yet exist.
pub async fn resolve_user_path(
    workspace_root: &Path,
    user_id: &str,
    relative: &str,
) -> Result<PathBuf, WorkspaceError> {
    let user_root = workspace_root.join(user_id);
    let joined = user_root.join(relative);
    let canonical = tokio::fs::canonicalize(&joined).await?;
    let user_root_canonical = tokio::fs::canonicalize(&user_root).await?;
    if !canonical.starts_with(&user_root_canonical) {
        return Err(WorkspaceError::Traversal(relative.into()));
    }
    Ok(canonical)
}

/// Same as `resolve_user_path`, but the final component(s) are permitted to not exist.
/// Canonicalizes the deepest existing ancestor and joins the remainder, validating that
/// no component uses `..` to escape after canonicalization.
pub async fn resolve_user_path_for_create(
    workspace_root: &Path,
    user_id: &str,
    relative: &str,
) -> Result<PathBuf, WorkspaceError> {
    let user_root = workspace_root.join(user_id);
    let user_root_canonical = tokio::fs::canonicalize(&user_root).await?;

    let joined = user_root.join(relative);
    let mut ancestor = joined.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !ancestor.exists() {
        tail.push(
            ancestor
                .file_name()
                .ok_or_else(|| WorkspaceError::Traversal(relative.into()))?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| WorkspaceError::Traversal(relative.into()))?;
    }
    let canonical_ancestor = tokio::fs::canonicalize(ancestor).await?;
    if !canonical_ancestor.starts_with(&user_root_canonical) {
        return Err(WorkspaceError::Traversal(relative.into()));
    }
    let mut result = canonical_ancestor;
    for component in tail.into_iter().rev() {
        if component == std::ffi::OsStr::new("..") || component == std::ffi::OsStr::new(".") {
            return Err(WorkspaceError::Traversal(relative.into()));
        }
        result.push(component);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_traversal_via_dotdot_rejected() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let victim_dir = root.path().join("bob");
        tokio::fs::create_dir_all(&victim_dir).await.unwrap();
        tokio::fs::write(victim_dir.join("secret.txt"), b"secret")
            .await
            .unwrap();

        let result = resolve_user_path(root.path(), "alice", "../bob/secret.txt").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_symlink_escape_rejected() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let outside = root.path().join("outside.txt");
        tokio::fs::write(&outside, b"outside").await.unwrap();
        tokio::fs::symlink(&outside, user_dir.join("escape"))
            .await
            .unwrap();

        let result = resolve_user_path(root.path(), "alice", "escape").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }

    #[tokio::test]
    async fn test_happy_path_resolves() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("MEMORY.md"), b"hi")
            .await
            .unwrap();

        let result = resolve_user_path(root.path(), "alice", "MEMORY.md")
            .await
            .unwrap();
        assert_eq!(result, user_dir.canonicalize().unwrap().join("MEMORY.md"));
    }
}

#[cfg(test)]
mod create_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_deep_path_allowed() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let result =
            resolve_user_path_for_create(root.path(), "alice", "skills/git/SKILL.md")
                .await
                .unwrap();
        let expected = user_dir
            .canonicalize()
            .unwrap()
            .join("skills")
            .join("git")
            .join("SKILL.md");
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn test_create_dotdot_rejected() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let result =
            resolve_user_path_for_create(root.path(), "alice", "../etc/passwd").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }
}
