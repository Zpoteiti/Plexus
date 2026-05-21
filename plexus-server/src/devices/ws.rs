use crate::{
    app::AppState,
    db::devices::{self, DeviceRow},
};
use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, header},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use plexus_common::{
    ErrorCode,
    protocol::{DeviceConfig, ErrorFrame, FsPolicy, HelloAckFrame, WsFrame},
    version::PROTOCOL_VERSION,
};
use tokio::sync::mpsc;

pub async fn device_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let token = bearer_token(&headers);
    ws.on_upgrade(move |socket| async move {
        run_socket(state, socket, token).await;
    })
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(ToOwned::to_owned)
}

async fn run_socket(state: AppState, mut socket: WebSocket, token: Option<String>) {
    let Some(token) = token else {
        close(&mut socket, 4401, r#"{"code":"unauthorized"}"#).await;
        return;
    };
    let row = match devices::find_by_token(state.pool(), &token).await {
        Ok(Some(row)) => row,
        Ok(None) | Err(_) => {
            close(&mut socket, 4401, r#"{"code":"unauthorized"}"#).await;
            return;
        }
    };

    let Some(Ok(Message::Text(text))) = socket.next().await else {
        close(&mut socket, 1002, "expected hello").await;
        return;
    };
    let Ok(WsFrame::Hello(hello)) = serde_json::from_str::<WsFrame>(&text) else {
        close(&mut socket, 1002, "expected hello").await;
        return;
    };
    if hello.version != PROTOCOL_VERSION {
        close(&mut socket, 4409, r#"{"code":"version_unsupported"}"#).await;
        return;
    }

    let ack = WsFrame::HelloAck(HelloAckFrame {
        id: hello.id,
        device_name: row.name.clone(),
        user_id: row.user_id,
        config: device_config_from_row(&row),
    });
    let text = serde_json::to_string(&ack).unwrap();
    if socket.send(Message::Text(text.into())).await.is_err() {
        return;
    }

    let (tx, mut rx) = mpsc::channel::<WsFrame>(32);
    let now = time::OffsetDateTime::now_utc();
    let handle = crate::devices::ConnHandle {
        token: row.token.clone(),
        user_id: row.user_id,
        device_name: row.name.clone(),
        connected_at: now,
        last_seen: now,
        tx,
    };
    let (generation, old) = state.devices().register(handle).await;
    if let Some(old) = old {
        let _ = old
            .tx
            .send(close_command_frame(crate::devices::CloseReason::Replaced))
            .await;
    }

    let (mut sender, mut receiver) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let text = serde_json::to_string(&frame).unwrap();
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = receiver.next().await {
        match message {
            Message::Text(text) => match serde_json::from_str::<WsFrame>(&text) {
                Ok(WsFrame::Pong(_)) | Ok(WsFrame::Error(_)) => {}
                _ => {}
            },
            Message::Close(_) => break,
            _ => {}
        }
    }
    writer.abort();
    state
        .devices()
        .unregister_if_current(&row.token, generation)
        .await;
}

pub fn device_config_from_row(row: &DeviceRow) -> DeviceConfig {
    DeviceConfig {
        workspace_path: row.workspace_path.clone(),
        fs_policy: fs_policy_from_row(&row.fs_policy),
        shell_timeout_max: row.shell_timeout_max as u32,
        ssrf_whitelist: serde_json::from_value(row.ssrf_whitelist.clone())
            .expect("stored device ssrf_whitelist must be Vec<String>"),
        mcp_servers: serde_json::from_value(row.mcp_servers.clone())
            .expect("stored device mcp_servers must be object of MCP server configs"),
    }
}

fn fs_policy_from_row(value: &str) -> FsPolicy {
    match value {
        "sandbox" => FsPolicy::Sandbox,
        "unrestricted" => FsPolicy::Unrestricted,
        other => panic!("stored device fs_policy must be valid, got {other:?}"),
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

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}
