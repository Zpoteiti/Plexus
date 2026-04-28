//! File-tool jail — `resolve_in_workspace` per ADR-073, ADR-105.
//!
//! Every shared file tool (read_file/write_file/edit_file/...) calls this
//! helper before any disk operation. Pure Rust path validation, no OS
//! primitive — works identically on Linux/macOS/Windows.

use crate::errors::WorkspaceError;
use std::path::{Path, PathBuf};

/// Resolve `path` (relative or absolute) against `workspace_root` and verify
/// the result stays inside `workspace_root` after canonicalization.
///
/// - Relative paths are joined onto `workspace_root`.
/// - Absolute paths are accepted as-is for validation.
/// - The path itself need NOT exist (so write_file can create new files).
///   The path's parent directory MUST exist — if it doesn't, returns
///   `WorkspaceError::NotFound(parent)`.
/// - Symlinks anywhere in the path are followed via `canonicalize()`. A
///   symlink that points outside the workspace fails the boundary check.
/// - The workspace root itself MUST exist; missing root returns
///   `WorkspaceError::NotFound(root)`.
///
/// Returns the canonicalized absolute path on success.
pub fn resolve_in_workspace(workspace_root: &Path, path: &str) -> Result<PathBuf, WorkspaceError> {
    if path.is_empty() {
        return Err(WorkspaceError::PathOutsideWorkspace(PathBuf::from(path)));
    }

    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|_| WorkspaceError::NotFound(workspace_root.to_path_buf()))?;

    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        canonical_root.join(path)
    };

    let resolved = match candidate.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // Path doesn't exist (e.g. write to a new file).
            // Canonicalize the parent and re-attach the basename;
            // this still catches symlink escapes via the parent.
            let parent = candidate
                .parent()
                .ok_or_else(|| WorkspaceError::PathOutsideWorkspace(candidate.clone()))?;
            match parent.canonicalize() {
                Ok(canonical_parent) => {
                    let basename = candidate
                        .file_name()
                        .ok_or_else(|| WorkspaceError::PathOutsideWorkspace(candidate.clone()))?;
                    canonical_parent.join(basename)
                }
                Err(_) => {
                    // Parent doesn't exist. Walk further up to find an
                    // existing ancestor. If that ancestor is outside the
                    // workspace, the path was an escape attempt — report
                    // PathOutsideWorkspace, not NotFound.
                    let mut cursor = parent;
                    let canonical_ancestor = loop {
                        match cursor.canonicalize() {
                            Ok(p) => break p,
                            Err(_) => match cursor.parent() {
                                Some(p) => cursor = p,
                                None => {
                                    return Err(WorkspaceError::PathOutsideWorkspace(
                                        candidate.clone(),
                                    ));
                                }
                            },
                        }
                    };
                    if !canonical_ancestor.starts_with(&canonical_root) {
                        return Err(WorkspaceError::PathOutsideWorkspace(candidate));
                    }
                    return Err(WorkspaceError::NotFound(parent.to_path_buf()));
                }
            }
        }
    };

    if !resolved.starts_with(&canonical_root) {
        return Err(WorkspaceError::PathOutsideWorkspace(resolved));
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::WorkspaceError;
    use std::fs;
    use tempfile::TempDir;

    fn workspace() -> TempDir {
        TempDir::new().expect("create tempdir")
    }

    #[test]
    fn relative_path_resolves_under_root() {
        let ws = workspace();
        let resolved = resolve_in_workspace(ws.path(), "MEMORY.md").unwrap();
        // Compare canonicalized — TempDir on macOS may use /private/var symlink.
        assert!(resolved.starts_with(ws.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("MEMORY.md"));
    }

    #[test]
    fn relative_subdir_resolves_under_root() {
        let ws = workspace();
        fs::create_dir(ws.path().join("subdir")).unwrap();
        let resolved = resolve_in_workspace(ws.path(), "subdir/file.txt").unwrap();
        assert!(resolved.ends_with("subdir/file.txt"));
    }

    #[test]
    fn absolute_path_inside_workspace_accepted() {
        let ws = workspace();
        let canonical = ws.path().canonicalize().unwrap();
        let inside = canonical.join("file.txt");
        let resolved = resolve_in_workspace(ws.path(), inside.to_str().unwrap()).unwrap();
        assert_eq!(resolved, inside);
    }

    #[test]
    fn absolute_path_outside_workspace_rejected() {
        let ws = workspace();
        let result = resolve_in_workspace(ws.path(), "/etc/passwd");
        assert!(matches!(
            result,
            Err(WorkspaceError::PathOutsideWorkspace(_))
        ));
    }

    #[test]
    fn dot_dot_traversal_rejected() {
        let ws = workspace();
        // ../etc/passwd resolves to ws.parent/etc/passwd which is outside ws.
        let result = resolve_in_workspace(ws.path(), "../etc/passwd");
        assert!(matches!(
            result,
            Err(WorkspaceError::PathOutsideWorkspace(_))
        ));
    }

    #[test]
    fn dot_dot_that_stays_inside_accepted() {
        let ws = workspace();
        fs::create_dir(ws.path().join("a")).unwrap();
        fs::create_dir(ws.path().join("b")).unwrap();
        // a/../b resolves to b which is inside.
        let resolved = resolve_in_workspace(ws.path(), "a/../b").unwrap();
        assert!(resolved.ends_with("b"));
    }

    #[test]
    fn empty_path_rejected() {
        let ws = workspace();
        let result = resolve_in_workspace(ws.path(), "");
        assert!(matches!(
            result,
            Err(WorkspaceError::PathOutsideWorkspace(_))
        ));
    }

    #[test]
    fn workspace_root_does_not_exist_rejected() {
        let result = resolve_in_workspace(
            std::path::Path::new("/nonexistent/totally/fake/dir"),
            "file.txt",
        );
        assert!(matches!(result, Err(WorkspaceError::NotFound(_))));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_pointing_outside_rejected() {
        use std::os::unix::fs::symlink;
        let ws = workspace();
        // Use the system temp_dir() as a target outside ws.
        let outside_target = std::env::temp_dir();
        let link = ws.path().join("escape");
        symlink(&outside_target, &link).unwrap();
        let result = resolve_in_workspace(ws.path(), "escape");
        assert!(matches!(
            result,
            Err(WorkspaceError::PathOutsideWorkspace(_))
        ));
    }

    #[test]
    fn nonexistent_file_under_workspace_accepted() {
        let ws = workspace();
        // For write_file we need to allow new file paths. Validation
        // checks the parent dir's canonicalization, not the file's.
        let resolved = resolve_in_workspace(ws.path(), "new_file_to_create.txt").unwrap();
        assert!(resolved.ends_with("new_file_to_create.txt"));
    }

    #[test]
    fn nonexistent_file_in_nonexistent_subdir_rejected() {
        let ws = workspace();
        // Parent must exist for validation. Don't auto-mkdir nested paths.
        let result = resolve_in_workspace(ws.path(), "no/such/dir/file.txt");
        assert!(matches!(result, Err(WorkspaceError::NotFound(_))));
    }

    #[test]
    fn workspace_root_itself_accepted() {
        let ws = workspace();
        let canonical = ws.path().canonicalize().unwrap();
        let resolved = resolve_in_workspace(ws.path(), canonical.to_str().unwrap()).unwrap();
        assert_eq!(resolved, canonical);
    }

    #[test]
    fn trailing_slash_handled() {
        let ws = workspace();
        fs::create_dir(ws.path().join("subdir")).unwrap();
        let resolved = resolve_in_workspace(ws.path(), "subdir/").unwrap();
        assert!(resolved.ends_with("subdir"));
    }

    #[test]
    fn deep_nested_path_under_workspace_accepted() {
        let ws = workspace();
        fs::create_dir_all(ws.path().join("a/b/c")).unwrap();
        let resolved = resolve_in_workspace(ws.path(), "a/b/c/file.txt").unwrap();
        let canonical = ws.path().canonicalize().unwrap();
        assert!(resolved.starts_with(&canonical));
    }

    #[test]
    fn absolute_path_to_workspace_parent_rejected() {
        let ws = workspace();
        let parent = ws.path().parent().unwrap();
        let result = resolve_in_workspace(ws.path(), parent.to_str().unwrap());
        assert!(matches!(
            result,
            Err(WorkspaceError::PathOutsideWorkspace(_))
        ));
    }
}
