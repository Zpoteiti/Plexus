use axum::{Json, http::StatusCode, response::IntoResponse};
use plexus_common::{Code, ErrorCode, WorkspaceError};
use serde::Serialize;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Serialize)]
struct ErrorBody {
    code: ErrorCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn from_sqlx(err: sqlx::Error) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::IoError,
            format!("database error: {err}"),
        )
    }

    pub fn invalid_args(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidArgs, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorBody {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

impl From<WorkspaceError> for ApiError {
    fn from(err: WorkspaceError) -> Self {
        let status = match &err {
            WorkspaceError::NotFound(_) => StatusCode::NOT_FOUND,
            WorkspaceError::PathOutsideWorkspace(_) => StatusCode::FORBIDDEN,
            WorkspaceError::SoftLocked | WorkspaceError::UploadTooLarge { .. } => {
                StatusCode::CONFLICT
            }
            WorkspaceError::QuotaNotConfigured => StatusCode::BAD_REQUEST,
            WorkspaceError::IoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = match &err {
            WorkspaceError::NotFound(_) => "workspace path not found".to_string(),
            WorkspaceError::PathOutsideWorkspace(_) => {
                "path resolves outside the workspace root".to_string()
            }
            WorkspaceError::IoError(_) => "workspace I/O error".to_string(),
            _ => err.to_string(),
        };
        Self::new(status, err.code(), message)
    }
}
