//! Workspace-layer errors.
//!
//! Covers path resolution (traversal/escape attempts), filesystem I/O, and
//! per-user quota enforcement. The quota variants were previously a separate
//! `QuotaError` type in `plexus-server`; they are folded here so there is one
//! typed error for everything the workspace layer can return.

use super::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// Relative path escapes the user's workspace root (via `..`, absolute
    /// path, or symlink). The inner string is the offending input path.
    #[error("path traversal attempt: {0}")]
    Traversal(String),

    /// Filesystem I/O failure (read/write/metadata/canonicalize).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Incoming upload exceeds the per-upload cap (80% of the user's quota).
    #[error("upload exceeds per-upload cap ({actual} bytes; cap {limit} bytes)")]
    UploadTooLarge { limit: u64, actual: u64 },

    /// The user's current usage is over quota; further uploads are refused
    /// until they delete files to bring usage back under the limit.
    #[error("workspace is soft-locked; delete files to continue")]
    SoftLocked,
}

impl WorkspaceError {
    pub fn code(&self) -> ErrorCode {
        match self {
            WorkspaceError::Traversal(_) => ErrorCode::Forbidden,
            WorkspaceError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
                ErrorCode::NotFound
            }
            WorkspaceError::Io(_) => ErrorCode::InternalError,
            WorkspaceError::UploadTooLarge { .. } => ErrorCode::ValidationFailed,
            WorkspaceError::SoftLocked => ErrorCode::Conflict,
        }
    }
}
