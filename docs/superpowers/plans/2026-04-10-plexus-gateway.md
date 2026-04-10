# plexus-gateway Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `plexus-gateway`, a stateless Rust binary that multiplexes browser WebSocket connections through a single server pipe, proxies REST, and serves the frontend.

**Architecture:** axum WebSocket server with DashMap for O(1) chat_id→browser routing, bounded mpsc channels with try_send for non-blocking delivery, CancellationToken for leak-free connection lifecycle, and reqwest for REST passthrough. Fully stateless w.r.t. sessions.

**Tech Stack:** Rust 2024 edition, axum 0.8 (ws), tokio 1, DashMap 6, jsonwebtoken 9, subtle 2, reqwest 0.12, tower-http 0.6, tokio-tungstenite 0.26 (tests only)

**Design Spec:** `docs/superpowers/specs/2026-04-10-m3-gateway-frontend-design.md` (r4)
**Protocol Spec:** `plexus-gateway/docs/PROTOCOL.md`

---

## File Structure

```
plexus-gateway/
├── Cargo.toml
├── .env.example
└── src/
    ├── main.rs           — bootstrap, router assembly, signal handler
    ├── lib.rs            — re-exports for integration tests
    ├── config.rs         — env loading via dotenvy, Config struct, origin parsing
    ├── state.rs          — AppState, BrowserConnection, OutboundFrame
    ├── jwt.rs            — JWT validation, Claims struct
    ├── proxy.rs          — /api/* REST passthrough with JWT gate
    ├── routing.rs        — chat_id → browser lookup, try_send dispatch
    ├── static_files.rs   — frontend serving with SPA fallback
    └── ws/
        ├── mod.rs        — re-exports
        ├── chat.rs       — /ws/chat browser handler (reader + writer + keepalive)
        └── plexus.rs     — /ws/plexus server handler (auth + reader + writer)
```

Also modify:
- `Cargo.toml` (workspace root, line 2) — add `plexus-gateway` to members

---

### Task 1: Scaffold the crate

**Files:**
- Modify: `Cargo.toml` (workspace root, line 2)
- Create: `plexus-gateway/Cargo.toml`
- Create: `plexus-gateway/.env.example`
- Create: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Add to workspace**

In the root `Cargo.toml`, change line 2:

```toml
members = ["plexus-common", "plexus-client", "plexus-server", "plexus-gateway"]
```

- [ ] **Step 2: Create gateway Cargo.toml**

Create `plexus-gateway/Cargo.toml`:

```toml
[package]
name = "plexus-gateway"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
axum = { version = "0.8", features = ["ws"] }
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
tower-http = { version = "0.6", features = ["cors", "fs", "trace", "limit"] }
futures-util = "0.3"
jsonwebtoken = "9"
subtle = "2"
dashmap = "6"
reqwest = { version = "0.12", features = ["json"] }
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
plexus-common = { path = "../plexus-common", features = ["axum"] }

[dev-dependencies]
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 3: Create .env.example**

Create `plexus-gateway/.env.example`:

```env
PLEXUS_GATEWAY_TOKEN=change-me-shared-secret
JWT_SECRET=change-me-jwt-secret
GATEWAY_PORT=9090
PLEXUS_SERVER_API_URL=http://localhost:8080
PLEXUS_FRONTEND_DIR=../plexus-frontend/dist
PLEXUS_ALLOWED_ORIGINS=*
```

- [ ] **Step 4: Create minimal main.rs**

Create `plexus-gateway/src/main.rs`:

```rust
use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let app = Router::new().route("/healthz", get(|| async { "ok" }));

    let listener = TcpListener::bind("0.0.0.0:9090").await.unwrap();
    tracing::info!("Gateway listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo build --package plexus-gateway`
Expected: builds successfully with no errors

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml plexus-gateway/
git commit -m "feat(gateway): scaffold plexus-gateway crate"
```

---

### Task 2: Config + State

**Files:**
- Create: `plexus-gateway/src/config.rs`
- Create: `plexus-gateway/src/state.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Write config.rs**

Create `plexus-gateway/src/config.rs`:

```rust
/// Gateway configuration loaded from environment variables.

#[derive(Debug, Clone)]
pub struct Config {
    pub gateway_token: String,
    pub jwt_secret: String,
    pub port: u16,
    pub server_api_url: String,
    pub frontend_dir: String,
    pub allowed_origins: AllowedOrigins,
}

#[derive(Debug, Clone)]
pub enum AllowedOrigins {
    Any,
    List(Vec<String>),
}

impl Config {
    /// Load config from environment. Panics on missing required vars.
    pub fn from_env() -> Self {
        Self {
            gateway_token: std::env::var("PLEXUS_GATEWAY_TOKEN")
                .expect("PLEXUS_GATEWAY_TOKEN required"),
            jwt_secret: std::env::var("JWT_SECRET").expect("JWT_SECRET required"),
            port: std::env::var("GATEWAY_PORT")
                .expect("GATEWAY_PORT required")
                .parse()
                .expect("GATEWAY_PORT must be a number"),
            server_api_url: std::env::var("PLEXUS_SERVER_API_URL")
                .expect("PLEXUS_SERVER_API_URL required"),
            frontend_dir: std::env::var("PLEXUS_FRONTEND_DIR")
                .unwrap_or_else(|_| "../plexus-frontend/dist".into()),
            allowed_origins: match std::env::var("PLEXUS_ALLOWED_ORIGINS")
                .unwrap_or_else(|_| "*".into())
                .as_str()
            {
                "*" => AllowedOrigins::Any,
                list => AllowedOrigins::List(
                    list.split(',').map(|s| s.trim().to_string()).collect(),
                ),
            },
        }
    }

    /// Check if the given origin is allowed.
    pub fn origin_allowed(&self, origin: Option<&str>) -> bool {
        match &self.allowed_origins {
            AllowedOrigins::Any => true,
            AllowedOrigins::List(list) => match origin {
                Some(o) => list.iter().any(|allowed| allowed == o),
                None => false, // strict mode requires an origin
            },
        }
    }
}
```

- [ ] **Step 2: Write state.rs**

Create `plexus-gateway/src/state.rs`:

```rust
/// Gateway application state — stateless w.r.t. sessions.

use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::config::Config;

pub struct AppState {
    pub config: Config,
    pub browsers: Arc<DashMap<String, BrowserConnection>>,
    pub plexus: Arc<RwLock<Option<mpsc::Sender<serde_json::Value>>>>,
    pub http_client: reqwest::Client,
    pub shutdown: CancellationToken,
}

#[derive(Clone)]
pub struct BrowserConnection {
    pub tx: mpsc::Sender<OutboundFrame>,
    pub user_id: String,
    pub cancel: CancellationToken,
}

pub enum OutboundFrame {
    Message(serde_json::Value),
    Progress(serde_json::Value),
    Error(serde_json::Value),
    Ping,
}
```

- [ ] **Step 3: Update main.rs to use config and state**

Replace `plexus-gateway/src/main.rs`:

```rust
mod config;
mod state;

use crate::config::Config;
use crate::state::AppState;
use axum::{Router, routing::get, extract::State, Json};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

async fn healthz(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let plexus_connected = state.plexus.read().await.is_some();
    let browsers = state.browsers.len();
    Json(serde_json::json!({
        "status": "ok",
        "plexus_connected": plexus_connected,
        "browsers": browsers,
    }))
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();
    let port = config.port;

    let state = Arc::new(AppState {
        config,
        browsers: Arc::new(DashMap::new()),
        plexus: Arc::new(RwLock::new(None)),
        http_client: reqwest::Client::new(),
        shutdown: CancellationToken::new(),
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state(state.clone());

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("Gateway listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build --package plexus-gateway`
Expected: builds successfully

- [ ] **Step 5: Commit**

```bash
git add plexus-gateway/src/
git commit -m "feat(gateway): add config and state modules"
```

---

### Task 3: JWT validation

**Files:**
- Create: `plexus-gateway/src/jwt.rs`
- Modify: `plexus-gateway/src/main.rs` (add `mod jwt;`)

- [ ] **Step 1: Write jwt.rs with implementation and tests**

Create `plexus-gateway/src/jwt.rs`:

```rust
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub is_admin: bool,
    pub exp: u64,
}

/// Validate a JWT and return the claims.
pub fn validate(token: &str, secret: &str) -> Result<Claims, String> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["sub", "exp"]);

    decode::<Claims>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(|e| format!("JWT validation failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    const SECRET: &str = "test-secret-key";

    fn make_token(sub: &str, is_admin: bool, exp: u64) -> String {
        let claims = Claims {
            sub: sub.to_string(),
            is_admin,
            exp,
        };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(SECRET.as_bytes()))
            .unwrap()
    }

    fn future_exp() -> u64 {
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs())
            + 3600
    }

    #[test]
    fn valid_token() {
        let token = make_token("user42", false, future_exp());
        let claims = validate(&token, SECRET).unwrap();
        assert_eq!(claims.sub, "user42");
        assert!(!claims.is_admin);
    }

    #[test]
    fn expired_token() {
        let token = make_token("user42", false, 1000);
        let result = validate(&token, SECRET);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn wrong_secret() {
        let token = make_token("user42", false, future_exp());
        let result = validate(&token, "wrong-secret");
        assert!(result.is_err());
    }

    #[test]
    fn malformed_token() {
        let result = validate("not.a.jwt", SECRET);
        assert!(result.is_err());
    }

    #[test]
    fn completely_garbage() {
        let result = validate("garbage", SECRET);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Add mod to main.rs**

Add `mod jwt;` to the top of `plexus-gateway/src/main.rs` (after `mod state;`).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --package plexus-gateway jwt`
Expected: all 5 tests pass

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/jwt.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add JWT validation with tests"
```

---

### Task 4: /ws/plexus handler

**Files:**
- Create: `plexus-gateway/src/ws/mod.rs`
- Create: `plexus-gateway/src/ws/plexus.rs`
- Create: `plexus-gateway/src/routing.rs` (stub)
- Modify: `plexus-gateway/src/main.rs` (add `mod ws;`, `mod routing;`, wire route)

- [ ] **Step 1: Create ws/mod.rs**

Create `plexus-gateway/src/ws/mod.rs`:

```rust
pub mod plexus;
pub mod chat;
```

- [ ] **Step 2: Write plexus.rs**

Create `plexus-gateway/src/ws/plexus.rs`:

```rust
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
```

- [ ] **Step 3: Create routing.rs stub**

Create `plexus-gateway/src/routing.rs`:

```rust
use crate::state::{AppState, OutboundFrame};
use std::sync::Arc;
use tokio::sync::mpsc;

pub enum RouteResult {
    DirectHit,
    NoMatch,
    Evicted,
}

pub fn route_send(state: &Arc<AppState>, msg: &serde_json::Value) -> RouteResult {
    let chat_id = msg.get("chat_id").and_then(|c| c.as_str()).unwrap_or("");
    tracing::warn!("routing stub: dropping message for chat_id={chat_id}");
    RouteResult::NoMatch
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
```

- [ ] **Step 4: Create a stub ws/chat.rs (needed for ws/mod.rs)**

Create `plexus-gateway/src/ws/chat.rs`:

```rust
// Full implementation in Task 5
```

- [ ] **Step 5: Update main.rs — add mods and wire route**

Add to top of `main.rs`:

```rust
mod ws;
mod routing;
```

Update the router:

```rust
let app = Router::new()
    .route("/healthz", get(healthz))
    .route("/ws/plexus", get(ws::plexus::ws_plexus))
    .with_state(state.clone());
```

- [ ] **Step 6: Run unit tests**

Run: `cargo test --package plexus-gateway`
Expected: all JWT + token verification tests pass

- [ ] **Step 7: Commit**

```bash
git add plexus-gateway/src/
git commit -m "feat(gateway): add /ws/plexus handler with auth and token verification"
```

---

### Task 5: /ws/chat handler

**Files:**
- Modify: `plexus-gateway/src/ws/chat.rs`
- Modify: `plexus-gateway/src/main.rs` (wire route)

- [ ] **Step 1: Write chat.rs**

Replace `plexus-gateway/src/ws/chat.rs` with:

```rust
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
```

- [ ] **Step 2: Wire route in main.rs**

Add to the router:

```rust
.route("/ws/chat", get(ws::chat::ws_chat))
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package plexus-gateway`
Expected: builds successfully

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/
git commit -m "feat(gateway): add /ws/chat handler with keepalive and session validation"
```

---

### Task 6: Routing (full implementation)

**Files:**
- Modify: `plexus-gateway/src/routing.rs`

- [ ] **Step 1: Write routing tests**

Add to the bottom of `plexus-gateway/src/routing.rs`:

```rust
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
        let (tx, _rx) = mpsc::channel(1); // buffer of 1
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
```

- [ ] **Step 2: Replace route_send stub with full implementation**

Replace the full content of `plexus-gateway/src/routing.rs` (keeping `forward_to_plexus` and tests):

```rust
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

// ... tests from Step 1 go here
```

- [ ] **Step 3: Run tests**

Run: `cargo test --package plexus-gateway routing`
Expected: all 5 routing tests pass

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/routing.rs
git commit -m "feat(gateway): implement routing with eviction and backpressure"
```

---

### Task 7: REST proxy

**Files:**
- Create: `plexus-gateway/src/proxy.rs`
- Modify: `plexus-gateway/src/main.rs` (add `mod proxy;`, wire route)

- [ ] **Step 1: Write proxy.rs**

Create `plexus-gateway/src/proxy.rs`:

```rust
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tracing::warn;

const MAX_RESPONSE_BYTES: usize = 25 * 1024 * 1024; // 25 MB

const HOP_BY_HOP: &[&str] = &[
    "host", "connection", "transfer-encoding", "upgrade",
    "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer",
];

pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
) -> Response {
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();

    // Path traversal check
    if path.contains("..") {
        return (StatusCode::UNPROCESSABLE_ENTITY, "path traversal not allowed").into_response();
    }

    // JWT validation — skip for /api/auth/*
    let is_public = path.starts_with("/api/auth/");
    if !is_public {
        let auth_header = req.headers().get("authorization").and_then(|v| v.to_str().ok());
        match auth_header {
            Some(h) if h.starts_with("Bearer ") => {
                let token = &h[7..];
                if let Err(e) = crate::jwt::validate(token, &state.config.jwt_secret) {
                    warn!("proxy: JWT validation failed: {e}");
                    return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
                }
            }
            _ => {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            }
        }
    }

    // Build upstream URL
    let upstream = format!("{}{}{}", state.config.server_api_url, path, query);

    // Copy headers, strip hop-by-hop
    let method = req.method().clone();
    let mut upstream_headers = reqwest::header::HeaderMap::new();
    for (key, value) in req.headers() {
        let name = key.as_str().to_lowercase();
        if !HOP_BY_HOP.contains(&name.as_str()) {
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                    upstream_headers.insert(name, val);
                }
            }
        }
    }

    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_RESPONSE_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response();
        }
    };

    let upstream_req = state
        .http_client
        .request(method, &upstream)
        .headers(upstream_headers)
        .body(body_bytes);

    let upstream_resp = match upstream_req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("proxy: upstream error: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                serde_json::json!({"error":{"code":"upstream_unreachable","message":e.to_string()}}).to_string(),
            ).into_response();
        }
    };

    let status = StatusCode::from_u16(upstream_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let resp_headers = upstream_resp.headers().clone();

    let resp_bytes = match upstream_resp.bytes().await {
        Ok(b) => {
            if b.len() > MAX_RESPONSE_BYTES {
                return (
                    StatusCode::BAD_GATEWAY,
                    serde_json::json!({"error":{"code":"upstream_too_large","message":"response body exceeded 25 MB limit"}}).to_string(),
                ).into_response();
            }
            b
        }
        Err(e) => {
            warn!("proxy: failed to read upstream response: {e}");
            return (StatusCode::BAD_GATEWAY, "upstream read error").into_response();
        }
    };

    let mut response = Response::builder().status(status);
    for (key, value) in &resp_headers {
        let name = key.as_str().to_lowercase();
        if !HOP_BY_HOP.contains(&name.as_str()) {
            response = response.header(key.as_str(), value.as_bytes());
        }
    }
    response.body(Body::from(resp_bytes)).unwrap()
}
```

- [ ] **Step 2: Wire in main.rs**

Add `mod proxy;` and update the router:

```rust
use axum::routing::any;

let app = Router::new()
    .route("/healthz", get(healthz))
    .route("/ws/chat", get(ws::chat::ws_chat))
    .route("/ws/plexus", get(ws::plexus::ws_plexus))
    .route("/api/{*rest}", any(proxy::proxy_handler))
    .layer(RequestBodyLimitLayer::new(25 * 1024 * 1024))
    .with_state(state.clone());
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package plexus-gateway`
Expected: builds successfully

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/proxy.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add REST proxy with JWT gate and body limits"
```

---

### Task 8: Static files + graceful shutdown

**Files:**
- Create: `plexus-gateway/src/static_files.rs`
- Modify: `plexus-gateway/src/main.rs` (final assembly)

- [ ] **Step 1: Write static_files.rs**

Create `plexus-gateway/src/static_files.rs`:

```rust
use tower_http::services::{ServeDir, ServeFile};

pub fn static_file_service(frontend_dir: &str) -> ServeDir<ServeFile> {
    let index = format!("{frontend_dir}/index.html");
    ServeDir::new(frontend_dir).fallback(ServeFile::new(index))
}
```

- [ ] **Step 2: Write final main.rs**

Replace `plexus-gateway/src/main.rs`:

```rust
mod config;
mod jwt;
mod proxy;
mod routing;
mod state;
mod static_files;
mod ws;

use crate::config::Config;
use crate::state::AppState;
use axum::{extract::State, routing::{any, get}, Json, Router};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;
use tracing_subscriber::EnvFilter;

async fn healthz(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let plexus_connected = state.plexus.read().await.is_some();
    let browsers = state.browsers.len();
    Json(serde_json::json!({
        "status": "ok",
        "plexus_connected": plexus_connected,
        "browsers": browsers,
    }))
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();
    let port = config.port;
    let frontend_dir = config.frontend_dir.clone();

    let state = Arc::new(AppState {
        config,
        browsers: Arc::new(DashMap::new()),
        plexus: Arc::new(RwLock::new(None)),
        http_client: reqwest::Client::new(),
        shutdown: CancellationToken::new(),
    });

    let static_service = static_files::static_file_service(&frontend_dir);

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/ws/chat", get(ws::chat::ws_chat))
        .route("/ws/plexus", get(ws::plexus::ws_plexus))
        .route("/api/{*rest}", any(proxy::proxy_handler))
        .fallback_service(static_service)
        .layer(RequestBodyLimitLayer::new(25 * 1024 * 1024))
        .with_state(state.clone());

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("Gateway listening on {}", listener.local_addr().unwrap());

    // Graceful shutdown
    let shutdown_state = state.clone();
    let shutdown_future = async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received");
        shutdown_state.shutdown.cancel();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_future)
        .await
        .unwrap();

    tracing::info!("Gateway shut down");
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --package plexus-gateway`
Expected: builds successfully

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/
git commit -m "feat(gateway): final assembly with static files and graceful shutdown"
```

---

### Task 9: Integration tests

**Files:**
- Create: `plexus-gateway/src/lib.rs`
- Modify: `plexus-gateway/src/main.rs` (use lib re-exports)
- Create: `plexus-gateway/tests/integration.rs`

- [ ] **Step 1: Create lib.rs for test access**

Create `plexus-gateway/src/lib.rs`:

```rust
pub mod config;
pub mod jwt;
pub mod proxy;
pub mod routing;
pub mod state;
pub mod static_files;
pub mod ws;
```

- [ ] **Step 2: Update main.rs to use lib re-exports**

Replace the `mod` declarations in `main.rs` with:

```rust
use plexus_gateway::{config::Config, state::AppState};
```

Remove all `mod` declarations from `main.rs`. The modules are now in `lib.rs`.

- [ ] **Step 3: Write integration test helpers**

Create `plexus-gateway/tests/integration.rs`:

```rust
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use plexus_gateway::config::{AllowedOrigins, Config};
use plexus_gateway::state::AppState;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use axum::{routing::{any, get}, extract::State, Json, Router};
use tower_http::limit::RequestBodyLimitLayer;

fn test_config(port: u16) -> Config {
    Config {
        gateway_token: "test-token".into(),
        jwt_secret: "test-secret".into(),
        port,
        server_api_url: "http://127.0.0.1:1".into(), // intentionally unreachable
        frontend_dir: ".".into(),
        allowed_origins: AllowedOrigins::Any,
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
    use jsonwebtoken::{encode, EncodingKey, Header};
    use plexus_gateway::jwt::Claims;
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() + 3600;
    let claims = Claims { sub: user_id.to_string(), is_admin: false, exp };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
}

#[tokio::test]
async fn test_healthz() {
    let (state, port) = start_gateway(test_config(0)).await;
    let resp = reqwest::get(format!("http://127.0.0.1:{port}/healthz")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["plexus_connected"], false);
    assert_eq!(body["browsers"], 0);
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_plexus_auth_ok() {
    let (state, port) = start_gateway(test_config(0)).await;
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus")).await.unwrap();
    ws.send(Message::Text(serde_json::json!({"type":"auth","token":"test-token"}).to_string().into())).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "auth_ok");
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_plexus_bad_token() {
    let (state, port) = start_gateway(test_config(0)).await;
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus")).await.unwrap();
    ws.send(Message::Text(serde_json::json!({"type":"auth","token":"wrong"}).to_string().into())).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "auth_fail");
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_plexus_duplicate() {
    let (state, port) = start_gateway(test_config(0)).await;
    // First connection
    let (mut ws1, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus")).await.unwrap();
    ws1.send(Message::Text(serde_json::json!({"type":"auth","token":"test-token"}).to_string().into())).await.unwrap();
    let _ = ws1.next().await; // auth_ok
    // Second connection — should be rejected
    let (mut ws2, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus")).await.unwrap();
    ws2.send(Message::Text(serde_json::json!({"type":"auth","token":"test-token"}).to_string().into())).await.unwrap();
    let resp = ws2.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "auth_fail");
    assert!(v["reason"].as_str().unwrap().contains("duplicate"));
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_browser_no_plexus() {
    let (state, port) = start_gateway(test_config(0)).await;
    let jwt = make_jwt("test-secret", "user1");
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}")).await.unwrap();
    // Send a message — no plexus connected
    ws.send(Message::Text(serde_json::json!({
        "type": "message",
        "session_id": "gateway:user1:sess1",
        "content": "hello",
    }).to_string().into())).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "error");
    assert!(v["reason"].as_str().unwrap().contains("not connected"));
    state.shutdown.cancel();
}

#[tokio::test]
async fn test_round_trip() {
    let (state, port) = start_gateway(test_config(0)).await;

    // Connect plexus
    let (mut plexus_ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus")).await.unwrap();
    plexus_ws.send(Message::Text(serde_json::json!({"type":"auth","token":"test-token"}).to_string().into())).await.unwrap();
    let _ = plexus_ws.next().await; // auth_ok

    // Connect browser
    let jwt = make_jwt("test-secret", "user1");
    let (mut browser_ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}")).await.unwrap();

    // Browser sends message
    browser_ws.send(Message::Text(serde_json::json!({
        "type": "message",
        "session_id": "gateway:user1:sess1",
        "content": "hello from browser",
    }).to_string().into())).await.unwrap();

    // Plexus should receive it
    let plexus_msg = plexus_ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&plexus_msg.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "hello from browser");
    let chat_id = v["chat_id"].as_str().unwrap().to_string();

    // Plexus sends reply
    plexus_ws.send(Message::Text(serde_json::json!({
        "type": "send",
        "chat_id": chat_id,
        "session_id": "gateway:user1:sess1",
        "content": "hello from agent",
    }).to_string().into())).await.unwrap();

    // Browser should receive it
    let browser_msg = browser_ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&browser_msg.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "hello from agent");

    state.shutdown.cancel();
}

#[tokio::test]
async fn test_invalid_session_id() {
    let (state, port) = start_gateway(test_config(0)).await;
    let jwt = make_jwt("test-secret", "user1");
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}")).await.unwrap();
    // Send message with wrong user_id in session_id
    ws.send(Message::Text(serde_json::json!({
        "type": "message",
        "session_id": "gateway:hacker:sess1",
        "content": "spoofed",
    }).to_string().into())).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp.into_text().unwrap()).unwrap();
    assert_eq!(v["type"], "error");
    assert!(v["reason"].as_str().unwrap().contains("session_id"));
    state.shutdown.cancel();
}
```

- [ ] **Step 4: Run all tests**

Run: `cargo test --package plexus-gateway`
Expected: all unit + integration tests pass

- [ ] **Step 5: Commit**

```bash
git add plexus-gateway/
git commit -m "feat(gateway): add integration tests for full WS round-trip"
```

---

## Validation Gate

After Task 9, manually verify with Postman + curl:

1. Start plexus-server: `cargo run --package plexus-server`
2. Start gateway: `cargo run --package plexus-gateway`
3. Curl healthz: `curl http://localhost:9090/healthz`
4. Curl REST proxy: `curl -H "Authorization: Bearer <JWT>" http://localhost:9090/api/sessions`
5. Connect Postman WS to `ws://localhost:9090/ws/chat?token=<JWT>`
6. Send a message, wait for agent response
7. Verify progress hints and final response arrive

**Phase 1 is complete when all tests pass and manual validation succeeds.**
