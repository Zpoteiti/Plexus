use crate::state::{AppState, BrowserConnection, OutboundFrame};
use axum::{
    extract::{Query, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    http::HeaderMap,
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn, info_span, Instrument};

#[derive(Deserialize)]
pub struct WsChatQuery {
    token: String,
}

pub async fn ws_chat(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<WsChatQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    // Origin check
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    if !state.config.origin_allowed(origin) {
        return Response::builder()
            .status(403)
            .body("Origin not allowed".into())
            .unwrap();
    }

    // JWT validation
    let claims = match crate::jwt::validate(&query.token, &state.config.jwt_secret) {
        Ok(c) => c,
        Err(e) => {
            warn!("ws_chat: JWT validation failed: {e}");
            return Response::builder()
                .status(401)
                .body("Unauthorized".into())
                .unwrap();
        }
    };

    let user_id = claims.sub;
    ws.on_upgrade(move |socket| handle_chat(socket, state, user_id))
}

async fn handle_chat(socket: WebSocket, state: Arc<AppState>, user_id: String) {
    let chat_id = uuid::Uuid::new_v4().to_string();
    let (mut sink, mut stream) = socket.split();

    // Create channel + cancel token
    let (tx, mut rx) = mpsc::channel::<OutboundFrame>(64);
    let conn_cancel = state.shutdown.child_token();

    // Insert into DashMap
    state.browsers.insert(
        chat_id.clone(),
        BrowserConnection {
            tx: tx.clone(),
            user_id: user_id.clone(),
            cancel: conn_cancel.clone(),
        },
    );

    let missed_pongs = Arc::new(AtomicU32::new(0));

    // Spawn writer task
    let writer_cancel = conn_cancel.clone();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = writer_cancel.cancelled() => break,
                Some(frame) = rx.recv() => {
                    let text = match frame {
                        OutboundFrame::Ping => "{\"type\":\"ping\"}".to_string(),
                        OutboundFrame::Message(v) | OutboundFrame::Progress(v) | OutboundFrame::Error(v) => {
                            v.to_string()
                        }
                    };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
        let _ = sink.close().await;
    });

    // Spawn keepalive task
    let ka_tx = tx.clone();
    let ka_cancel = conn_cancel.clone();
    let ka_pongs = missed_pongs.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = ka_cancel.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
            }
            if ka_pongs.load(Ordering::Relaxed) > 3 {
                warn!("keepalive: missed too many pongs, cancelling");
                ka_cancel.cancel();
                break;
            }
            if ka_tx.try_send(OutboundFrame::Ping).is_err() {
                warn!("keepalive: channel full, cancelling");
                ka_cancel.cancel();
                break;
            }
            ka_pongs.fetch_add(1, Ordering::Relaxed);
        }
    });

    // Reader loop
    let span = info_span!("ws_chat", %chat_id, %user_id);
    async {
        info!("Browser connected");
        loop {
            tokio::select! {
                biased;
                _ = conn_cancel.cancelled() => break,
                msg = stream.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            let parsed: serde_json::Value = match serde_json::from_str(&text) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match msg_type {
                                "message" => {
                                    handle_browser_message(&state, &chat_id, &user_id, &parsed, &tx).await;
                                }
                                "pong" => {
                                    missed_pongs.store(0, Ordering::Relaxed);
                                }
                                _ => {
                                    warn!("unknown message type: {msg_type}");
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
        state.browsers.remove(&chat_id);
        conn_cancel.cancel();
        drop(tx);
        let _ = writer.await;
        info!("Browser disconnected");
    }
    .instrument(span)
    .await;
}

async fn handle_browser_message(
    state: &Arc<AppState>,
    chat_id: &str,
    user_id: &str,
    msg: &serde_json::Value,
    tx: &mpsc::Sender<OutboundFrame>,
) {
    let session_id = msg.get("session_id").and_then(|s| s.as_str()).unwrap_or("");
    let expected_prefix = format!("gateway:{user_id}:");

    if !session_id.starts_with(&expected_prefix) {
        let _ = tx.try_send(OutboundFrame::Error(serde_json::json!({
            "type": "error",
            "reason": "invalid session_id"
        })));
        return;
    }

    crate::routing::forward_to_plexus(state, chat_id, user_id, msg, tx).await;
}
