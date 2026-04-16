use crate::state::AppState;
use axum::{
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub async fn ws_plexus(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_plexus(socket, state))
}

async fn handle_plexus(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Wait for auth message (5s timeout)
    let auth_msg = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.next(),
    )
    .await;

    let auth_json = match auth_msg {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => v,
                Err(_) => {
                    let _ = send_json(&mut sink, &serde_json::json!({"type":"auth_fail","reason":"invalid JSON"})).await;
                    return;
                }
            }
        }
        _ => {
            let _ = send_json(&mut sink, &serde_json::json!({"type":"auth_fail","reason":"timeout or invalid frame"})).await;
            return;
        }
    };

    // Verify auth type and token
    let msg_type = auth_json.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if msg_type != "auth" {
        let _ = send_json(&mut sink, &serde_json::json!({"type":"auth_fail","reason":"expected auth message"})).await;
        return;
    }

    let provided_token = auth_json.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if !verify_token(provided_token, &state.config.gateway_token) {
        let _ = send_json(&mut sink, &serde_json::json!({"type":"auth_fail","reason":"invalid token"})).await;
        return;
    }

    // Check for duplicate connection
    let (plexus_tx, mut plexus_rx) = mpsc::channel::<serde_json::Value>(256);
    {
        let mut guard = state.plexus.write().await;
        if guard.is_some() {
            drop(guard);
            let _ = send_json(&mut sink, &serde_json::json!({"type":"auth_fail","reason":"duplicate connection"})).await;
            return;
        }
        *guard = Some(plexus_tx);
    }

    let _ = send_json(&mut sink, &serde_json::json!({"type":"auth_ok"})).await;
    info!("Plexus server connected");

    // Spawn writer task
    let plexus_cancel = state.shutdown.child_token();
    let writer_cancel = plexus_cancel.clone();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = writer_cancel.cancelled() => break,
                Some(msg) = plexus_rx.recv() => {
                    let text = serde_json::to_string(&msg).unwrap_or_default();
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
        let _ = sink.close().await;
    });

    // Reader loop
    loop {
        tokio::select! {
            biased;
            _ = plexus_cancel.cancelled() => break,
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let parsed: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match msg_type {
                            "send" => {
                                crate::routing::route_send(&state, &parsed);
                            }
                            "session_update" => {
                                let user_id = parsed.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
                                let session_id = parsed.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
                                crate::routing::route_session_update(&state, user_id, session_id);
                            }
                            _ => {
                                warn!("ws_plexus: unknown message type: {msg_type}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => continue,
                }
            }
        }
    }

    // Cleanup
    {
        let mut guard = state.plexus.write().await;
        *guard = None;
    }
    plexus_cancel.cancel();
    let _ = writer.await;
    info!("Plexus server disconnected");
}

fn verify_token(provided: &str, expected: &str) -> bool {
    let a = provided.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

async fn send_json(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    value: &serde_json::Value,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(value).unwrap_or_default();
    sink.send(Message::Text(text.into())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_match() {
        assert!(verify_token("secret123", "secret123"));
    }

    #[test]
    fn token_mismatch_same_length() {
        assert!(!verify_token("secret123", "secret124"));
    }

    #[test]
    fn token_mismatch_different_length() {
        assert!(!verify_token("short", "longsecret"));
    }

    #[test]
    fn token_empty() {
        assert!(!verify_token("", "notempty"));
    }
}
