//! Errors raised by `workspace_fs` (server) and the file-tool jail (both
//! crates). See ADR-046, ADR-073.

use crate::errors::{Code, ErrorCode};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("path not found: {0}")]
    NotFound(PathBuf),

    #[error("workspace is over quota; only deletes are allowed until usage drops")]
    SoftLocked,

    #[error("upload size {actual_bytes} exceeds 80% of quota ({quota_bytes} bytes)")]
    UploadTooLarge { actual_bytes: u64, quota_bytes: u64 },

    #[error("path {0} resolves outside the workspace root")]
    PathOutsideWorkspace(PathBuf),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}

impl Code for WorkspaceError {
    fn code(&self) -> ErrorCode {
        match self {
            WorkspaceError::NotFound(_) => ErrorCode::NotFound,
            WorkspaceError::SoftLocked => ErrorCode::SoftLocked,
            WorkspaceError::UploadTooLarge { .. } => ErrorCode::UploadTooLarge,
            WorkspaceError::PathOutsideWorkspace(_) => ErrorCode::PathOutsideWorkspace,
            WorkspaceError::IoError(_) => ErrorCode::IoError,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_maps_to_not_found_code() {
        let e = WorkspaceError::NotFound(PathBuf::from("/some/path"));
        assert_eq!(e.code(), ErrorCode::NotFound);
    }

    #[test]
    fn soft_locked_maps() {
        assert_eq!(WorkspaceError::SoftLocked.code(), ErrorCode::SoftLocked);
    }

    #[test]
    fn upload_too_large_maps() {
        let e = WorkspaceError::UploadTooLarge {
            actual_bytes: 1000,
            quota_bytes: 800,
        };
        assert_eq!(e.code(), ErrorCode::UploadTooLarge);
    }

    #[test]
    fn path_outside_workspace_maps() {
        let e = WorkspaceError::PathOutsideWorkspace(PathBuf::from("/etc/passwd"));
        assert_eq!(e.code(), ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn io_error_maps() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
        let e: WorkspaceError = io_err.into();
        assert_eq!(e.code(), ErrorCode::IoError);
    }

    #[test]
    fn display_includes_path_for_not_found() {
        let e = WorkspaceError::NotFound(PathBuf::from("/foo/bar"));
        assert_eq!(format!("{}", e), "path not found: /foo/bar");
    }
}
