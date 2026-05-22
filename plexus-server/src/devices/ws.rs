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
    protocol::{DeviceConfig, FsPolicy, HelloAckFrame, PingFrame, WsFrame},
    version::PROTOCOL_VERSION,
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

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
    if load_device_or_close(&state, &mut socket, &token)
        .await
        .is_none()
    {
        return;
    }

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

    let Some(row) = load_device_or_close(&state, &mut socket, &token).await else {
        return;
    };

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

    let (tx, mut rx) = mpsc::channel::<crate::devices::registry::DeviceCommand>(32);
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
            .send(crate::devices::registry::DeviceCommand::Close(
                crate::devices::CloseReason::Replaced,
            ))
            .await;
    }

    let (mut sender, mut receiver) = socket.split();
    let (close_sent_tx, mut close_sent_rx) = oneshot::channel();
    let writer = tokio::spawn(async move {
        let mut close_sent_tx = Some(close_sent_tx);
        while let Some(command) = rx.recv().await {
            match command {
                crate::devices::registry::DeviceCommand::Frame(frame) => {
                    let text = serde_json::to_string(&frame).unwrap();
                    if sender.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                crate::devices::registry::DeviceCommand::Close(reason) => {
                    let (code, body) = close_payload(reason);
                    let _ = sender
                        .send(Message::Close(Some(CloseFrame {
                            code,
                            reason: body.into(),
                        })))
                        .await;
                    if let Some(close_sent_tx) = close_sent_tx.take() {
                        let _ = close_sent_tx.send(());
                    }
                    break;
                }
            }
        }
    });

    let (heartbeat_interval, heartbeat_missed_limit) = state.devices().heartbeat_config().await;
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now(), heartbeat_interval);
    let mut awaiting_pong: Option<Uuid> = None;
    let mut missed: u8 = 0;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if awaiting_pong.is_some() {
                    missed += 1;
                    if missed >= heartbeat_missed_limit {
                        state
                            .devices()
                            .close(&row.token, crate::devices::CloseReason::HeartbeatTimeout)
                            .await;
                        break;
                    }
                }
                let id = Uuid::now_v7();
                awaiting_pong = Some(id);
                let _ = state
                    .devices()
                    .send(&row.token, WsFrame::Ping(PingFrame { id }))
                    .await;
            }
            msg = receiver.next() => {
                let Some(Ok(message)) = msg else {
                    break;
                };
                match message {
                    Message::Text(text) => match serde_json::from_str::<WsFrame>(&text) {
                        Ok(WsFrame::Pong(pong)) if Some(pong.id) == awaiting_pong => {
                            awaiting_pong = None;
                            missed = 0;
                        }
                        Ok(WsFrame::Pong(_)) => {
                            send_error(
                                &state,
                                &row.token,
                                ErrorCode::MalformedFrame,
                                "unexpected pong",
                            )
                            .await;
                        }
                        Ok(WsFrame::Error(_)) => {}
                        Ok(_) => {
                            send_error(
                                &state,
                                &row.token,
                                ErrorCode::UnknownType,
                                "unsupported client frame",
                            )
                            .await;
                        }
                        Err(_) => {
                            send_error(
                                &state,
                                &row.token,
                                ErrorCode::MalformedFrame,
                                "malformed JSON frame",
                            )
                            .await;
                        }
                    },
                    Message::Binary(_) => {
                        send_error(
                            &state,
                            &row.token,
                            ErrorCode::MalformedFrame,
                            "binary frames are unsupported",
                        )
                        .await;
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = &mut close_sent_rx => {
                break;
            }
        }
    }
    state
        .devices()
        .unregister_if_current(&row.token, generation)
        .await;
    let _ = writer.await;
}

async fn load_device_or_close(
    state: &AppState,
    socket: &mut WebSocket,
    token: &str,
) -> Option<DeviceRow> {
    match devices::find_by_token(state.pool(), token).await {
        Ok(Some(row)) => Some(row),
        Ok(None) => {
            close(socket, 4401, r#"{"code":"unauthorized"}"#).await;
            None
        }
        Err(_) => {
            close(socket, 1013, r#"{"code":"io_error"}"#).await;
            None
        }
    }
}

async fn send_error(state: &AppState, token: &str, code: ErrorCode, message: &'static str) {
    let _ = state
        .devices()
        .send(
            token,
            WsFrame::Error(plexus_common::protocol::ErrorFrame {
                id: None,
                code,
                message: message.to_string(),
            }),
        )
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

fn close_payload(reason: crate::devices::CloseReason) -> (u16, String) {
    match reason {
        crate::devices::CloseReason::Replaced => (1000, String::new()),
        crate::devices::CloseReason::Unauthorized => {
            (4401, r#"{"code":"unauthorized"}"#.to_string())
        }
        crate::devices::CloseReason::HeartbeatTimeout => (4408, String::new()),
    }
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}
