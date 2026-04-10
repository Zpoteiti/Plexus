use crate::state::{AppState, BrowserConnection, OutboundFrame};
use std::sync::Arc;
use tokio::sync::mpsc;

pub enum RouteResult {
    DirectHit,
    NoMatch,
    Evicted,
}

pub fn route_send(state: &Arc<AppState>, msg: &serde_json::Value) -> RouteResult {
    let chat_id = match msg.get("chat_id").and_then(|c| c.as_str()) {
        Some(id) => id,
        None => {
            tracing::warn!("routing: message has no chat_id");
            return RouteResult::NoMatch;
        }
    };

    let is_progress = msg
        .get("metadata")
        .and_then(|m| m.get("_progress"))
        .and_then(|p| p.as_bool())
        .unwrap_or(false);

    let content = msg.get("content").cloned().unwrap_or(serde_json::Value::Null);
    let session_id = msg.get("session_id").cloned().unwrap_or(serde_json::Value::Null);

    let outbound = if is_progress {
        serde_json::json!({"type": "progress", "session_id": session_id, "content": content})
    } else {
        serde_json::json!({"type": "message", "session_id": session_id, "content": content})
    };

    let frame = if is_progress {
        OutboundFrame::Progress(outbound)
    } else {
        OutboundFrame::Message(outbound)
    };

    // Clone connection out of DashMap (shard guard dropped immediately)
    let conn = state.browsers.get(chat_id).map(|r| r.clone());

    match conn {
        Some(conn) => try_dispatch(state, chat_id, conn, frame),
        None => {
            tracing::warn!("routing: no browser for chat_id={chat_id}");
            RouteResult::NoMatch
        }
    }
}

fn try_dispatch(
    state: &Arc<AppState>,
    chat_id: &str,
    conn: BrowserConnection,
    frame: OutboundFrame,
) -> RouteResult {
    match &frame {
        OutboundFrame::Progress(_) => {
            let _ = conn.tx.try_send(frame);
            RouteResult::DirectHit
        }
        OutboundFrame::Message(_) => match conn.tx.try_send(frame) {
            Ok(()) => RouteResult::DirectHit,
            Err(_) => {
                tracing::warn!("evicting slow browser chat_id={chat_id}");
                state.browsers.remove(chat_id);
                conn.cancel.cancel();
                RouteResult::Evicted
            }
        },
        OutboundFrame::Error(_) | OutboundFrame::Ping => {
            let _ = conn.tx.try_send(frame);
            RouteResult::DirectHit
        }
    }
}

pub async fn forward_to_plexus(
    state: &Arc<AppState>,
    chat_id: &str,
    user_id: &str,
    msg: &serde_json::Value,
    tx: &mpsc::Sender<OutboundFrame>,
) {
    let plexus = state.plexus.read().await;
    let Some(plexus_tx) = plexus.as_ref() else {
        let _ = tx.try_send(OutboundFrame::Error(serde_json::json!({
            "type": "error",
            "reason": "Plexus server not connected"
        })));
        return;
    };

    let content = msg.get("content").cloned().unwrap_or(serde_json::Value::Null);
    let session_id = msg.get("session_id").cloned().unwrap_or(serde_json::Value::Null);
    let media = msg.get("media").cloned();

    let mut forwarded = serde_json::json!({
        "type": "message",
        "chat_id": chat_id,
        "sender_id": user_id,
        "session_id": session_id,
        "content": content,
    });
    if let Some(media) = media {
        forwarded["media"] = media;
    }

    if plexus_tx.try_send(forwarded).is_err() {
        let _ = tx.try_send(OutboundFrame::Error(serde_json::json!({
            "type": "error",
            "reason": "Plexus server busy"
        })));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, AllowedOrigins};
    use crate::state::{AppState, BrowserConnection, OutboundFrame};
    use dashmap::DashMap;
    use tokio::sync::{mpsc, RwLock};
    use tokio_util::sync::CancellationToken;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState {
            config: Config {
                gateway_token: "test".into(),
                jwt_secret: "test".into(),
                port: 0,
                server_api_url: "http://localhost".into(),
                frontend_dir: ".".into(),
                allowed_origins: AllowedOrigins::Any,
            },
            browsers: Arc::new(DashMap::new()),
            plexus: Arc::new(RwLock::new(None)),
            http_client: reqwest::Client::new(),
            shutdown: CancellationToken::new(),
        })
    }

    fn insert_browser(state: &Arc<AppState>, chat_id: &str) -> mpsc::Receiver<OutboundFrame> {
        let (tx, rx) = mpsc::channel(64);
        state.browsers.insert(
            chat_id.to_string(),
            BrowserConnection {
                tx,
                user_id: "user1".into(),
                cancel: CancellationToken::new(),
            },
        );
        rx
    }

    #[test]
    fn direct_hit_message() {
        let state = test_state();
        let mut rx = insert_browser(&state, "chat-1");
        let msg = serde_json::json!({
            "type": "send",
            "chat_id": "chat-1",
            "session_id": "gateway:user1:sess1",
            "content": "hello",
        });
        let result = route_send(&state, &msg);
        assert!(matches!(result, RouteResult::DirectHit));
        let frame = rx.try_recv().unwrap();
        assert!(matches!(frame, OutboundFrame::Message(_)));
    }

    #[test]
    fn direct_hit_progress() {
        let state = test_state();
        let mut rx = insert_browser(&state, "chat-2");
        let msg = serde_json::json!({
            "type": "send",
            "chat_id": "chat-2",
            "content": "thinking...",
            "metadata": {"_progress": true},
        });
        let result = route_send(&state, &msg);
        assert!(matches!(result, RouteResult::DirectHit));
        let frame = rx.try_recv().unwrap();
        assert!(matches!(frame, OutboundFrame::Progress(_)));
    }

    #[test]
    fn no_match() {
        let state = test_state();
        let msg = serde_json::json!({
            "type": "send",
            "chat_id": "nonexistent",
            "content": "hello",
        });
        let result = route_send(&state, &msg);
        assert!(matches!(result, RouteResult::NoMatch));
    }

    #[test]
    fn evict_slow_browser() {
        let state = test_state();
        let (tx, _rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        state.browsers.insert(
            "chat-slow".to_string(),
            BrowserConnection { tx, user_id: "user1".into(), cancel: cancel.clone() },
        );
        // Fill the channel
        let fill_msg = serde_json::json!({"type":"send","chat_id":"chat-slow","content":"fill"});
        route_send(&state, &fill_msg);
        // Next message should evict
        let evict_msg = serde_json::json!({"type":"send","chat_id":"chat-slow","content":"evict"});
        let result = route_send(&state, &evict_msg);
        assert!(matches!(result, RouteResult::Evicted));
        assert!(state.browsers.get("chat-slow").is_none());
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn progress_dropped_on_full() {
        let state = test_state();
        let (tx, _rx) = mpsc::channel(1);
        state.browsers.insert(
            "chat-full".to_string(),
            BrowserConnection { tx, user_id: "user1".into(), cancel: CancellationToken::new() },
        );
        // Fill the channel
        let fill = serde_json::json!({"type":"send","chat_id":"chat-full","content":"fill"});
        route_send(&state, &fill);
        // Progress should be silently dropped, NOT evicted
        let progress = serde_json::json!({"type":"send","chat_id":"chat-full","content":"thinking","metadata":{"_progress":true}});
        let result = route_send(&state, &progress);
        assert!(matches!(result, RouteResult::DirectHit));
        assert!(state.browsers.get("chat-full").is_some()); // NOT evicted
    }
}
