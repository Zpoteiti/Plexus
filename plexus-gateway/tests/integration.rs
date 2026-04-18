use axum::{
    Json, Router,
    extract::State,
    routing::{any, get},
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use plexus_gateway::config::{AllowedOrigins, Config};
use plexus_gateway::state::AppState;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;

fn test_config() -> Config {
    Config {
        gateway_token: "test-token".into(),
        jwt_secret: "test-secret".into(),
        port: 0,
        server_api_url: "http://127.0.0.1:1".into(), // intentionally unreachable
        frontend_dir: ".".into(),
        allowed_origins: AllowedOrigins::Any,
        upload_max_bytes: 1024 * 1024 * 1024,
    }
}

async fn healthz(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let plexus_connected = state.plexus.read().await.is_some();
    let browsers = state.browsers.len();
    Json(serde_json::json!({"status":"ok","plexus_connected":plexus_connected,"browsers":browsers}))
}

async fn start_gateway(config: Config) -> (Arc<AppState>, u16) {
    let state = Arc::new(AppState {
        config,
        browsers: Arc::new(DashMap::new()),
        plexus: Arc::new(RwLock::new(None)),
        http_client: reqwest::Client::new(),
        shutdown: CancellationToken::new(),
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/ws/chat", get(plexus_gateway::ws::chat::ws_chat))
        .route("/ws/plexus", get(plexus_gateway::ws::plexus::ws_plexus))
        .route("/api/{*rest}", any(plexus_gateway::proxy::proxy_handler))
        .layer(RequestBodyLimitLayer::new(25 * 1024 * 1024))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let shutdown_state = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_state.shutdown.cancelled().await;
            })
            .await
            .ok();
    });

    // Wait for server to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (state, port)
}

fn make_jwt(secret: &str, user_id: &str) -> String {
    use jsonwebtoken::{EncodingKey, Header, encode};
    use plexus_gateway::jwt::Claims;
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let claims = Claims {
        sub: user_id.to_string(),
        is_admin: false,
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

#[tokio::test]
async fn test_healthz() {
    let (state, port) = start_gateway(test_config()).await;
    let resp = reqwest::get(format!("http://127.0.0.1:{port}/healthz"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["plexus_connected"], false);
    assert_eq!(body["browsers"], 0);
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_plexus_auth_ok() {
    let (state, port) = start_gateway(test_config()).await;
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    ws.send(Message::Text(
        serde_json::json!({"type":"auth","token":"test-token"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "auth_ok");
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_plexus_bad_token() {
    let (state, port) = start_gateway(test_config()).await;
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    ws.send(Message::Text(
        serde_json::json!({"type":"auth","token":"wrong"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "auth_fail");
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_plexus_duplicate() {
    let (state, port) = start_gateway(test_config()).await;
    // First connection
    let (mut ws1, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    ws1.send(Message::Text(
        serde_json::json!({"type":"auth","token":"test-token"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let _ = ws1.next().await; // auth_ok
    // Second connection — should be rejected
    let (mut ws2, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    ws2.send(Message::Text(
        serde_json::json!({"type":"auth","token":"test-token"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let resp = ws2.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "auth_fail");
    assert!(v["reason"].as_str().unwrap().contains("duplicate"));
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_browser_no_plexus() {
    let (state, port) = start_gateway(test_config()).await;
    let jwt = make_jwt("test-secret", "user1");
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}"))
        .await
        .unwrap();
    // Send a message — no plexus connected
    ws.send(Message::Text(
        serde_json::json!({
            "type": "message",
            "session_id": "gateway:user1:sess1",
            "content": "hello",
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "error");
    assert!(v["reason"].as_str().unwrap().contains("not connected"));
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_round_trip() {
    let (state, port) = start_gateway(test_config()).await;

    // Connect plexus
    let (mut plexus_ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    plexus_ws
        .send(Message::Text(
            serde_json::json!({"type":"auth","token":"test-token"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let _ = plexus_ws.next().await; // auth_ok

    // Connect browser
    let jwt = make_jwt("test-secret", "user1");
    let (mut browser_ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}"))
        .await
        .unwrap();

    // Browser sends message
    browser_ws
        .send(Message::Text(
            serde_json::json!({
                "type": "message",
                "session_id": "gateway:user1:sess1",
                "content": "hello from browser",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // Plexus should receive it
    let plexus_msg = plexus_ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&plexus_msg.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "hello from browser");
    let chat_id = v["chat_id"].as_str().unwrap().to_string();

    // Plexus sends reply
    plexus_ws
        .send(Message::Text(
            serde_json::json!({
                "type": "send",
                "chat_id": chat_id,
                "session_id": "gateway:user1:sess1",
                "content": "hello from agent",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // Browser should receive it
    let browser_msg = browser_ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&browser_msg.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "hello from agent");

    state.shutdown.cancel();
}

#[tokio::test]
async fn test_invalid_session_id() {
    let (state, port) = start_gateway(test_config()).await;
    let jwt = make_jwt("test-secret", "user1");
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}"))
        .await
        .unwrap();
    // Send message with wrong user_id in session_id
    ws.send(Message::Text(
        serde_json::json!({
            "type": "message",
            "session_id": "gateway:hacker:sess1",
            "content": "spoofed",
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "error");
    assert!(v["reason"].as_str().unwrap().contains("session_id"));
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_progress_forwarding() {
    let (state, port) = start_gateway(test_config()).await;

    // Connect plexus
    let (mut plexus_ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    plexus_ws
        .send(Message::Text(
            serde_json::json!({"type":"auth","token":"test-token"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let _ = plexus_ws.next().await; // auth_ok

    // Connect browser
    let jwt = make_jwt("test-secret", "user1");
    let (mut browser_ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}"))
        .await
        .unwrap();

    // Browser sends message to get a chat_id
    browser_ws
        .send(Message::Text(
            serde_json::json!({
                "type": "message",
                "session_id": "gateway:user1:sess1",
                "content": "question",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let plexus_msg = plexus_ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&plexus_msg.into_text().unwrap()).unwrap();
    let chat_id = v["chat_id"].as_str().unwrap().to_string();

    // Plexus sends progress hint
    plexus_ws
        .send(Message::Text(
            serde_json::json!({
                "type": "send",
                "chat_id": chat_id,
                "session_id": "gateway:user1:sess1",
                "content": "thinking...",
                "metadata": {"_progress": true},
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // Browser should receive progress type
    let browser_msg = browser_ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&browser_msg.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "progress");
    assert_eq!(v["content"], "thinking...");

    state.shutdown.cancel();
}
