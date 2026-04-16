//! Gateway WebSocket client channel.
//! Connects to plexus-gateway, authenticates, forwards messages.
//! Stub for now — full implementation in M4 when gateway exists.

use crate::bus::{self, InboundEvent, OutboundEvent};
use crate::state::AppState;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Spawn the gateway connection loop. Reconnects with exponential backoff.
pub fn spawn_gateway_client(state: Arc<AppState>) {
    let ws_url = state.config.gateway_ws_url.clone();
    let token = state.config.gateway_token.clone();

    tokio::spawn(async move {
        let mut backoff = 1u64;
        loop {
            info!("Gateway: connecting to {ws_url}...");
            match connect_and_run(&state, &ws_url, &token).await {
                Ok(()) => {
                    info!("Gateway: connection closed cleanly");
                    backoff = 1;
                }
                Err(e) => {
                    warn!("Gateway: connection error: {e}");
                }
            }
            info!("Gateway: reconnecting in {backoff}s...");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
        }
    });
}

async fn connect_and_run(state: &Arc<AppState>, ws_url: &str, token: &str) -> Result<(), String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let (ws, _) = connect_async(ws_url)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    // Authenticate
    let auth = serde_json::json!({ "type": "auth", "token": token });
    sink.send(Message::Text(serde_json::to_string(&auth).unwrap().into()))
        .await
        .map_err(|e| format!("send auth: {e}"))?;

    // Await auth response
    match stream.next().await {
        Some(Ok(Message::Text(text))) => {
            let msg: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("parse: {e}"))?;
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("auth_ok") => info!("Gateway: authenticated"),
                Some("auth_fail") => {
                    let reason = msg
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("unknown");
                    return Err(format!("auth failed: {reason}"));
                }
                _ => return Err(format!("unexpected auth response: {text}")),
            }
        }
        _ => return Err("no auth response".into()),
    }

    // Store sink for outbound delivery
    let sink = Arc::new(tokio::sync::Mutex::new(sink));
    // Register gateway sink in state for outbound delivery
    // (Using a simple approach: store in a dedicated field or use the existing outbound channel)
    // For now, store as a gateway-specific sink
    {
        let mut gw = state.gateway_sink.write().await;
        *gw = Some(sink.clone());
    }

    // Message loop: receive messages from gateway
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else {
            continue;
        };
        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if msg_type != "message" {
            continue;
        }

        let content = parsed
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let chat_id = parsed
            .get("chat_id")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
        let sender_id = parsed
            .get("sender_id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let session_id = parsed
            .get("session_id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        let media = extract_media(&parsed);

        if session_id.is_empty() {
            continue;
        }
        if content.is_empty() && media.is_empty() {
            continue;
        }

        // The sender_id from gateway is the user_id (JWT-authenticated)
        let user_id = sender_id.clone().unwrap_or_default();

        // Gateway users are always the partner (JWT-authenticated)
        let event = InboundEvent {
            session_id,
            user_id: user_id.clone(),
            content,
            channel: plexus_common::consts::CHANNEL_GATEWAY.to_string(),
            chat_id,
            media,
            cron_job_id: None,
            identity: None, // Gateway = always partner, built from User in agent_loop
            metadata: Default::default(),
        };

        if let Err(e) = bus::publish_inbound(state, event).await {
            error!("Gateway inbound error: {e}");
        }
    }

    // Clear gateway sink on disconnect
    {
        let mut gw = state.gateway_sink.write().await;
        *gw = None;
    }

    Ok(())
}

fn extract_media(parsed: &serde_json::Value) -> Vec<String> {
    parsed
        .get("media")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_media_from_gateway_frame() {
        let raw = r#"{
            "type": "message",
            "chat_id": "c1",
            "sender_id": "u1",
            "session_id": "s1",
            "content": "hi",
            "media": ["/api/files/a", "/api/files/b"]
        }"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let media = extract_media(&parsed);
        assert_eq!(media, vec!["/api/files/a".to_string(), "/api/files/b".to_string()]);
    }

    #[test]
    fn test_parse_media_missing() {
        let raw = r#"{"type":"message","chat_id":"c","session_id":"s","content":"hi"}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(extract_media(&parsed).is_empty());
    }

    #[test]
    fn test_parse_media_malformed() {
        let raw = r#"{"type":"message","media":"not-an-array"}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(extract_media(&parsed).is_empty());
    }

    #[test]
    fn test_deliver_produces_session_update_frame() {
        let event = crate::bus::OutboundEvent {
            channel: "gateway".into(),
            chat_id: Some("stale-chat-id".into()),
            session_id: "session-123".into(),
            user_id: "user-alice".into(),
            content: "hello".into(),
            media: vec![],
            is_progress: false,
            metadata: Default::default(),
        };
        let frame = build_deliver_frame(&event);
        assert_eq!(frame["type"], "session_update");
        assert_eq!(frame["user_id"], "user-alice");
        assert_eq!(frame["session_id"], "session-123");
        // Chat_id must NOT leak into the outbound frame — it's a stale per-connect UUID.
        assert!(frame.get("chat_id").is_none());
        assert!(frame.get("content").is_none());
        assert!(frame.get("media").is_none());
    }
}

/// Deliver an outbound event to the gateway.
pub async fn deliver(state: &AppState, event: &OutboundEvent) {
    let sink = state.gateway_sink.read().await;
    let Some(sink) = sink.as_ref() else {
        warn!("Gateway: not connected, dropping outbound message");
        return;
    };
    let msg = build_deliver_frame(event);
    let json = serde_json::to_string(&msg).unwrap();
    let mut s = sink.lock().await;
    if let Err(e) = futures_util::SinkExt::send(
        &mut *s,
        tokio_tungstenite::tungstenite::Message::Text(json.into()),
    )
    .await
    {
        warn!("Gateway: send failed: {e}");
    }
}

/// Build the WS frame sent to plexus-gateway. Always a session_update
/// pointer — browsers fetch content via the existing REST history endpoint.
fn build_deliver_frame(event: &crate::bus::OutboundEvent) -> serde_json::Value {
    serde_json::json!({
        "type": "session_update",
        "user_id": event.user_id,
        "session_id": event.session_id,
    })
}
