use crate::{app::AppState, db::devices::DeviceRow, error::ApiError};
use axum::response::Response;
use plexus_common::{
    ErrorCode,
    protocol::{DeviceConfig, ErrorFrame, FsPolicy, WsFrame},
};

pub async fn device_ws(_state: AppState) -> Result<Response, ApiError> {
    Err(ApiError::invalid_args("device websocket not implemented"))
}

pub fn device_config_from_row(row: &DeviceRow) -> DeviceConfig {
    DeviceConfig {
        workspace_path: row.workspace_path.clone(),
        fs_policy: if row.fs_policy == "unrestricted" {
            FsPolicy::Unrestricted
        } else {
            FsPolicy::Sandbox
        },
        shell_timeout_max: row.shell_timeout_max as u32,
        ssrf_whitelist: serde_json::from_value(row.ssrf_whitelist.clone()).unwrap_or_default(),
        mcp_servers: serde_json::from_value(row.mcp_servers.clone()).unwrap_or_default(),
    }
}

pub fn close_command_frame(reason: crate::devices::CloseReason) -> WsFrame {
    let (code, message) = match reason {
        crate::devices::CloseReason::Replaced => {
            (ErrorCode::ClientShuttingDown, "connection replaced")
        }
        crate::devices::CloseReason::Unauthorized => (ErrorCode::Unauthorized, "unauthorized"),
        crate::devices::CloseReason::HeartbeatTimeout => {
            (ErrorCode::DeviceUnreachable, "heartbeat timeout")
        }
    };
    WsFrame::Error(ErrorFrame {
        id: None,
        code,
        message: message.to_string(),
    })
}
