# M3: Gateway + Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `plexus-gateway` (Rust WebSocket hub + REST proxy + static file server) and `plexus-frontend` (React 19 SPA with Chat / Settings / Admin pages).

**Architecture:** The gateway is a stateless "pipe with auth" — browsers and plexus-server both dial in as WebSocket clients, and the gateway routes messages between them by `chat_id`. Session state lives at plexus-server (DB). The frontend is a React SPA styled as "Cyberpunk Refined" (GitHub-dark base + neon green `#39ff14` accents) served by the gateway as static files in production.

**Tech Stack:**
- Gateway: Rust 2024 edition, axum 0.8 (with `ws` feature), jsonwebtoken, subtle, dashmap, reqwest, tower-http, tokio, dotenvy, tracing.
- Frontend: React 19, TypeScript 5.9, Vite 8, Tailwind CSS 4, Zustand 5, react-router-dom 7, react-markdown + remark-gfm, react-syntax-highlighter, lucide-react, Vitest.

**Spec:** `docs/superpowers/specs/2026-04-10-m3-gateway-frontend-design.md`

**Delivery Phases:**
- **Phase 1 (Tasks 1–10):** Gateway. User validates with Postman before Phase 2 starts.
- **Phase 2 (Tasks 11–24):** Frontend.

---

## File Map

### Phase 1 — plexus-gateway

| File | Responsibility |
|---|---|
| `Cargo.toml` (workspace) | Add `plexus-gateway` member |
| `plexus-gateway/Cargo.toml` | Crate deps |
| `plexus-gateway/.env.example` | Env var template |
| `plexus-gateway/src/main.rs` | Entry point, router, axum serve |
| `plexus-gateway/src/config.rs` | `Config` struct, env var loading |
| `plexus-gateway/src/state.rs` | `AppState` with DashMap and RwLock<Option<mpsc::Sender>> |
| `plexus-gateway/src/jwt.rs` | JWT validation using `jsonwebtoken` |
| `plexus-gateway/src/routing.rs` | Browser lookup by chat_id with sender_id fallback |
| `plexus-gateway/src/proxy.rs` | REST `/api/*` reverse proxy |
| `plexus-gateway/src/static_files.rs` | Frontend static file serving with SPA fallback |
| `plexus-gateway/src/ws/mod.rs` | Shared WS types, `BrowserConnection`, `OutboundFrame` |
| `plexus-gateway/src/ws/chat.rs` | `/ws/chat` browser handler |
| `plexus-gateway/src/ws/plexus.rs` | `/ws/plexus` plexus-server handler |
| `plexus-gateway/tests/integration.rs` | End-to-end tests (browser + plexus mocks) |

### Phase 2 — plexus-frontend

| File | Responsibility |
|---|---|
| `plexus-frontend/package.json` | npm deps + scripts |
| `plexus-frontend/vite.config.ts` | Vite config with `/api` and `/ws` proxy |
| `plexus-frontend/tailwind.config.ts` | Tailwind v4 config (theme tokens) |
| `plexus-frontend/tsconfig.json` | TypeScript config |
| `plexus-frontend/index.html` | HTML entry |
| `plexus-frontend/src/main.tsx` | Router bootstrap |
| `plexus-frontend/src/App.tsx` | Route guard wrapper |
| `plexus-frontend/src/lib/api.ts` | Fetch wrapper with JWT header + 401 handling |
| `plexus-frontend/src/lib/ws.ts` | Singleton WebSocket manager with reconnect |
| `plexus-frontend/src/lib/types.ts` | TS types mirroring server responses |
| `plexus-frontend/src/store/auth.ts` | Zustand auth store |
| `plexus-frontend/src/store/chat.ts` | Zustand chat store (sessions, messages, progress) |
| `plexus-frontend/src/store/devices.ts` | Zustand devices store (polled) |
| `plexus-frontend/src/pages/Login.tsx` | Login form |
| `plexus-frontend/src/pages/Chat.tsx` | Chat page (empty + active state) |
| `plexus-frontend/src/pages/Settings.tsx` | Tabbed settings (Profile / Devices / Channels / Skills / Cron) |
| `plexus-frontend/src/pages/Admin.tsx` | Tabbed admin (LLM / Soul / Rate / Server MCP) |
| `plexus-frontend/src/components/Sidebar.tsx` | Slim collapsible session list |
| `plexus-frontend/src/components/MessageList.tsx` | Scrollable message history |
| `plexus-frontend/src/components/Message.tsx` | Single message bubble |
| `plexus-frontend/src/components/ProgressHint.tsx` | Ephemeral tool progress indicator |
| `plexus-frontend/src/components/ChatInput.tsx` | Responsive textarea + send button |
| `plexus-frontend/src/components/DeviceStatusBar.tsx` | Top-bar status dots |
| `plexus-frontend/src/components/MarkdownContent.tsx` | react-markdown wrapper |
| `plexus-frontend/src/styles/globals.css` | Tailwind base + theme CSS vars |

---

## Phase 1 — plexus-gateway

### Task 1: Crate Scaffold + Workspace Integration

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `plexus-gateway/Cargo.toml`
- Create: `plexus-gateway/.env.example`
- Create: `plexus-gateway/src/main.rs` (stub)

- [ ] **Step 1: Add `plexus-gateway` to workspace members**

Edit root `Cargo.toml`:

```toml
[workspace]
members = ["plexus-common", "plexus-client", "plexus-server", "plexus-gateway"]
resolver = "2"
```

- [ ] **Step 2: Create `plexus-gateway/Cargo.toml`**

```toml
[package]
name = "plexus-gateway"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
plexus-common = { path = "../plexus-common", features = ["axum"] }
axum = { version = "0.8", features = ["ws"] }
tokio = { version = "1", features = ["full"] }
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
chrono = { version = "0.4", features = ["serde"] }

[dev-dependencies]
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
jsonwebtoken = "9"
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 3: Create `plexus-gateway/.env.example`**

```bash
# Shared secret for plexus-server auth (constant-time compared)
PLEXUS_GATEWAY_TOKEN=change-me-to-a-long-random-string

# HMAC secret for browser JWT validation (must match plexus-server JWT_SECRET)
JWT_SECRET=change-me-to-a-long-random-string

# Listen port
GATEWAY_PORT=9090

# Upstream plexus-server base URL for REST proxy
PLEXUS_SERVER_API_URL=http://localhost:3030

# Frontend static files directory (served as fallback route)
PLEXUS_FRONTEND_DIR=../plexus-frontend/dist
```

- [ ] **Step 4: Create stub `plexus-gateway/src/main.rs`**

```rust
fn main() {
    println!("plexus-gateway starting...");
}
```

- [ ] **Step 5: Build the workspace to verify the crate compiles**

Run: `cargo build --package plexus-gateway`
Expected: Compiles cleanly. May warn about unused dependencies; that's fine — subsequent tasks will use them.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml plexus-gateway/
git commit -m "feat(gateway): scaffold plexus-gateway crate with deps"
```

---

### Task 2: Config Module

**Files:**
- Create: `plexus-gateway/src/config.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Create `config.rs`**

```rust
//! Gateway configuration loaded from env vars at startup.

use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub gateway_token: String,
    pub jwt_secret: String,
    pub port: u16,
    pub plexus_server_api_url: String,
    pub frontend_dir: String,
}

impl Config {
    /// Load from environment variables. Panics if any required var is missing.
    pub fn from_env() -> Self {
        Self {
            gateway_token: required("PLEXUS_GATEWAY_TOKEN"),
            jwt_secret: required("JWT_SECRET"),
            port: required("GATEWAY_PORT")
                .parse()
                .expect("GATEWAY_PORT must be a valid u16"),
            plexus_server_api_url: required("PLEXUS_SERVER_API_URL"),
            frontend_dir: env::var("PLEXUS_FRONTEND_DIR")
                .unwrap_or_else(|_| "../plexus-frontend/dist".to_string()),
        }
    }
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("Required env var {name} is not set"))
}
```

- [ ] **Step 2: Wire config loading into `main.rs`**

Replace `plexus-gateway/src/main.rs`:

```rust
mod config;

use config::Config;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    tracing::info!("plexus-gateway config loaded: port={}", config.port);
    tracing::info!("upstream: {}", config.plexus_server_api_url);
    tracing::info!("frontend_dir: {}", config.frontend_dir);
}
```

- [ ] **Step 3: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build.

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/config.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add config loading from env vars"
```

---

### Task 3: JWT Validation Module

**Files:**
- Create: `plexus-gateway/src/jwt.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Write failing tests first**

Create `plexus-gateway/src/jwt.rs`:

```rust
//! JWT validation for browser WebSocket and REST requests.
//!
//! Mirrors plexus-server's `Claims` struct so tokens are interchangeable.

use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub is_admin: bool,
    pub exp: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("invalid or expired token: {0}")]
    Invalid(String),
}

/// Validate a JWT and return its claims. Uses HS256 with the shared secret.
pub fn validate(token: &str, secret: &str) -> Result<Claims, JwtError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|e| JwtError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    fn sign(claims: &Claims, secret: &str) -> String {
        encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn valid_token_returns_claims() {
        let secret = "test-secret";
        let claims = Claims {
            sub: "user-123".to_string(),
            is_admin: false,
            exp: chrono::Utc::now().timestamp() + 3600,
        };
        let token = sign(&claims, secret);
        let out = validate(&token, secret).unwrap();
        assert_eq!(out.sub, "user-123");
        assert!(!out.is_admin);
    }

    #[test]
    fn wrong_secret_fails() {
        let claims = Claims {
            sub: "user-123".to_string(),
            is_admin: false,
            exp: chrono::Utc::now().timestamp() + 3600,
        };
        let token = sign(&claims, "secret-a");
        assert!(validate(&token, "secret-b").is_err());
    }

    #[test]
    fn expired_token_fails() {
        let claims = Claims {
            sub: "user-123".to_string(),
            is_admin: false,
            exp: chrono::Utc::now().timestamp() - 10,
        };
        let token = sign(&claims, "s");
        assert!(validate(&token, "s").is_err());
    }

    #[test]
    fn malformed_token_fails() {
        assert!(validate("not.a.token", "s").is_err());
    }
}
```

- [ ] **Step 2: Add `thiserror` to `plexus-gateway/Cargo.toml`**

Add under `[dependencies]`:

```toml
thiserror = "2"
```

- [ ] **Step 3: Declare `jwt` module in `main.rs`**

Add to the module list in `plexus-gateway/src/main.rs`:

```rust
mod config;
mod jwt;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --package plexus-gateway jwt::`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add plexus-gateway/src/jwt.rs plexus-gateway/src/main.rs plexus-gateway/Cargo.toml
git commit -m "feat(gateway): add JWT validation with tests"
```

---

### Task 4: State Module (AppState + BrowserConnection)

**Files:**
- Create: `plexus-gateway/src/state.rs`
- Create: `plexus-gateway/src/ws/mod.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Create `ws/mod.rs` with shared types**

```rust
//! WebSocket shared types for browser and plexus-server connections.

use serde_json::Value;
use tokio::sync::mpsc;

/// A frame queued for delivery to a browser. Progress frames may be dropped
/// under backpressure; final Message frames must deliver or the connection
/// is closed.
#[derive(Debug, Clone)]
pub enum OutboundFrame {
    Message(Value),
    Progress(Value),
}

/// Per-browser handle held in `AppState.browsers`.
/// The real sink is owned by a dedicated writer task; other tasks send
/// frames through `outbound` which is a bounded channel.
#[derive(Debug, Clone)]
pub struct BrowserConnection {
    pub outbound: mpsc::Sender<OutboundFrame>,
    pub user_id: String,
}

pub mod chat;
pub mod plexus;
```

Leave `ws/chat.rs` and `ws/plexus.rs` as empty stub files for now — they will be created in later tasks:

```rust
//! /ws/chat browser WebSocket handler.
```

```rust
//! /ws/plexus server WebSocket handler.
```

- [ ] **Step 2: Create `state.rs`**

```rust
//! Shared application state.

use crate::config::Config;
use crate::ws::BrowserConnection;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub struct AppState {
    pub config: Config,
    /// chat_id → browser handle
    pub browsers: Arc<DashMap<String, BrowserConnection>>,
    /// Sender for the single plexus-server connection. `None` when not connected.
    pub plexus: Arc<RwLock<Option<mpsc::Sender<Value>>>>,
    /// Pooled HTTP client for REST proxy requests.
    pub http_client: reqwest::Client,
}

impl AppState {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            config,
            browsers: Arc::new(DashMap::new()),
            plexus: Arc::new(RwLock::new(None)),
            http_client: reqwest::Client::builder()
                .pool_max_idle_per_host(32)
                .build()
                .expect("reqwest client build"),
        })
    }
}
```

- [ ] **Step 3: Wire into `main.rs`**

Replace the module list and add state construction:

```rust
mod config;
mod jwt;
mod state;
mod ws;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    tracing::info!("plexus-gateway config loaded: port={}", config.port);
    let _state = AppState::new(config);
    tracing::info!("state initialized");
}
```

- [ ] **Step 4: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build.

- [ ] **Step 5: Commit**

```bash
git add plexus-gateway/src/state.rs plexus-gateway/src/ws/ plexus-gateway/src/main.rs
git commit -m "feat(gateway): add AppState with DashMap and WS types"
```

---

### Task 5: Static File Serving with SPA Fallback

**Files:**
- Create: `plexus-gateway/src/static_files.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Create `static_files.rs`**

```rust
//! Serves the built frontend from `PLEXUS_FRONTEND_DIR` with SPA fallback:
//! unknown paths return `index.html` so the React router can handle them.

use axum::Router;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

/// Build a service that serves the frontend dist/ directory. SPA fallback
/// points to `index.html` inside the same dir. If the dir doesn't exist at
/// startup, requests return 404 — the operator should build the frontend first.
pub fn service<S>(frontend_dir: &str) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let dir = PathBuf::from(frontend_dir);
    let index = dir.join("index.html");
    let serve_dir = ServeDir::new(&dir).fallback(ServeFile::new(index));
    Router::new().fallback_service(serve_dir)
}
```

- [ ] **Step 2: Build a minimal router in `main.rs`**

Replace `plexus-gateway/src/main.rs`:

```rust
mod config;
mod jwt;
mod state;
mod static_files;
mod ws;

use axum::Router;
use config::Config;
use state::AppState;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    let state = AppState::new(config.clone());

    let app: Router = Router::new()
        .merge(static_files::service(&config.frontend_dir))
        .with_state(Arc::clone(&state));

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("plexus-gateway listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 3: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build.

- [ ] **Step 4: Smoke-test the server**

Create a temporary test frontend dir:

```bash
mkdir -p /tmp/plexus-gateway-test/dist
echo '<h1>plexus-frontend placeholder</h1>' > /tmp/plexus-gateway-test/dist/index.html
```

Create `plexus-gateway/.env`:

```bash
PLEXUS_GATEWAY_TOKEN=test-token-12345
JWT_SECRET=test-jwt-secret
GATEWAY_PORT=9090
PLEXUS_SERVER_API_URL=http://localhost:3030
PLEXUS_FRONTEND_DIR=/tmp/plexus-gateway-test/dist
```

Run the gateway in one terminal:

```bash
cargo run --package plexus-gateway
```

In another terminal:

```bash
curl -sv http://localhost:9090/
curl -sv http://localhost:9090/some/spa/route
```

Expected: Both return the placeholder `<h1>` HTML (SPA fallback).

Kill the gateway (Ctrl+C).

- [ ] **Step 5: Commit**

```bash
git add plexus-gateway/src/static_files.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): serve frontend static files with SPA fallback"
```

---

### Task 6: Routing Module (chat_id → browser lookup)

**Files:**
- Create: `plexus-gateway/src/routing.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Write failing tests first**

Create `plexus-gateway/src/routing.rs`:

```rust
//! Routes messages from plexus-server to browser connections.
//!
//! Primary: lookup by `chat_id` in `state.browsers`.
//! Fallback: if `chat_id` not found, route to any open browser for
//! `metadata.sender_id` (handles cron-triggered pushes where the original
//! chat_id is stale).

use crate::state::AppState;
use crate::ws::{BrowserConnection, OutboundFrame};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::warn;

/// Result of a routing decision.
#[derive(Debug, PartialEq, Eq)]
pub enum RouteResult {
    /// Delivered via direct chat_id lookup.
    DirectHit,
    /// Delivered via sender_id fallback.
    SenderFallback,
    /// No matching browser connection found.
    NoMatch,
}

/// Route a `send` message from plexus-server to the correct browser.
pub async fn route_send(state: &Arc<AppState>, msg: &Value) -> RouteResult {
    let chat_id = msg.get("chat_id").and_then(|v| v.as_str()).unwrap_or("");
    let content = msg.get("content").cloned().unwrap_or(Value::Null);
    let metadata = msg.get("metadata").cloned().unwrap_or(json!({}));
    let session_id = msg.get("session_id").cloned().unwrap_or(Value::Null);

    let is_progress = metadata
        .get("_progress")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let media = metadata.get("media").cloned();

    let frame_type = if is_progress { "progress" } else { "message" };
    let mut outbound = json!({
        "type": frame_type,
        "content": content,
        "session_id": session_id,
    });
    if let Some(media) = media {
        outbound["media"] = media;
    }

    let frame = if is_progress {
        OutboundFrame::Progress(outbound)
    } else {
        OutboundFrame::Message(outbound)
    };

    // Direct lookup
    if let Some(conn) = state.browsers.get(chat_id) {
        if try_send(&conn, frame.clone()).await {
            return RouteResult::DirectHit;
        }
    }

    // Fallback: any browser connection for the given sender_id
    if let Some(sender_id) = metadata.get("sender_id").and_then(|v| v.as_str()) {
        let candidates: Vec<BrowserConnection> = state
            .browsers
            .iter()
            .filter(|entry| entry.value().user_id == sender_id)
            .map(|entry| entry.value().clone())
            .collect();
        for conn in candidates {
            if try_send(&conn, frame.clone()).await {
                return RouteResult::SenderFallback;
            }
        }
    }

    warn!("routing: no match for chat_id={chat_id}");
    RouteResult::NoMatch
}

async fn try_send(conn: &BrowserConnection, frame: OutboundFrame) -> bool {
    match &frame {
        OutboundFrame::Progress(_) => {
            // Drop progress hints if the channel is full — they are ephemeral.
            conn.outbound.try_send(frame).is_ok()
        }
        OutboundFrame::Message(_) => {
            // Blocking send for final messages — if this errors, the writer
            // task is dead and the entry will be cleaned up on disconnect.
            conn.outbound.send(frame).await.is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tokio::sync::mpsc;

    fn test_state() -> Arc<AppState> {
        let config = Config {
            gateway_token: "t".into(),
            jwt_secret: "s".into(),
            port: 0,
            plexus_server_api_url: "http://localhost".into(),
            frontend_dir: "/tmp".into(),
        };
        AppState::new(config)
    }

    fn register_browser(state: &Arc<AppState>, chat_id: &str, user_id: &str)
        -> mpsc::Receiver<OutboundFrame>
    {
        let (tx, rx) = mpsc::channel(8);
        state.browsers.insert(
            chat_id.to_string(),
            BrowserConnection {
                outbound: tx,
                user_id: user_id.to_string(),
            },
        );
        rx
    }

    #[tokio::test]
    async fn direct_chat_id_hit() {
        let state = test_state();
        let mut rx = register_browser(&state, "chat-1", "user-1");
        let msg = json!({
            "type": "send",
            "chat_id": "chat-1",
            "content": "hello",
        });
        let result = route_send(&state, &msg).await;
        assert_eq!(result, RouteResult::DirectHit);
        let frame = rx.recv().await.unwrap();
        match frame {
            OutboundFrame::Message(v) => {
                assert_eq!(v["type"], "message");
                assert_eq!(v["content"], "hello");
            }
            _ => panic!("expected Message"),
        }
    }

    #[tokio::test]
    async fn sender_id_fallback() {
        let state = test_state();
        let mut rx = register_browser(&state, "chat-existing", "user-42");
        let msg = json!({
            "type": "send",
            "chat_id": "chat-stale",
            "content": "scheduled task result",
            "metadata": { "sender_id": "user-42" },
        });
        let result = route_send(&state, &msg).await;
        assert_eq!(result, RouteResult::SenderFallback);
        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn no_match_returns_nomatch() {
        let state = test_state();
        let msg = json!({
            "type": "send",
            "chat_id": "nope",
            "content": "void",
        });
        assert_eq!(route_send(&state, &msg).await, RouteResult::NoMatch);
    }

    #[tokio::test]
    async fn progress_frame_sets_type() {
        let state = test_state();
        let mut rx = register_browser(&state, "chat-p", "user-p");
        let msg = json!({
            "type": "send",
            "chat_id": "chat-p",
            "content": "Executing shell on laptop...",
            "metadata": { "_progress": true },
        });
        assert_eq!(route_send(&state, &msg).await, RouteResult::DirectHit);
        match rx.recv().await.unwrap() {
            OutboundFrame::Progress(v) => assert_eq!(v["type"], "progress"),
            _ => panic!("expected Progress"),
        }
    }
}
```

- [ ] **Step 2: Register `routing` module in `main.rs`**

```rust
mod config;
mod jwt;
mod routing;
mod state;
mod static_files;
mod ws;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --package plexus-gateway routing::`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/routing.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add routing module with chat_id lookup + fallback"
```

---

### Task 7: Browser WebSocket Handler (`/ws/chat`)

**Files:**
- Replace: `plexus-gateway/src/ws/chat.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Implement browser handler**

Replace `plexus-gateway/src/ws/chat.rs`:

```rust
//! /ws/chat browser WebSocket handler.
//!
//! Flow:
//! 1. Validate JWT from `token` query param (401 before upgrade on failure).
//! 2. Assign chat_id + initial session_id, insert into state.browsers.
//! 3. Spawn a writer task owning the sink; send frames through a bounded mpsc channel.
//! 4. Read loop dispatches incoming JSON by `type`.

use crate::jwt;
use crate::state::AppState;
use crate::ws::{BrowserConnection, OutboundFrame};
use axum::extract::{
    Query, State, WebSocketUpgrade,
    ws::{Message, WebSocket},
};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Deserialize)]
pub struct ChatQuery {
    pub token: String,
}

pub async fn handler(
    ws: WebSocketUpgrade,
    Query(params): Query<ChatQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let claims = match jwt::validate(&params.token, &state.config.jwt_secret) {
        Ok(c) => c,
        Err(e) => {
            warn!("ws/chat: JWT rejected: {e}");
            return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
        }
    };
    let user_id = claims.sub.clone();
    ws.on_upgrade(move |socket| run(socket, state, user_id))
}

async fn run(socket: WebSocket, state: Arc<AppState>, user_id: String) {
    let chat_id = uuid::Uuid::new_v4().to_string();
    let initial_session = format!("gateway:{}:{}", user_id, uuid::Uuid::new_v4());

    let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundFrame>(64);

    state.browsers.insert(
        chat_id.clone(),
        BrowserConnection {
            outbound: outbound_tx.clone(),
            user_id: user_id.clone(),
        },
    );
    info!(
        "ws/chat: browser connected chat_id={chat_id} user_id={user_id}"
    );

    let (ws_sink, mut ws_stream) = socket.split();

    // Writer task: owns the sink and drains outbound_rx
    let writer = tokio::spawn(writer_task(ws_sink, outbound_rx));

    // Send initial session_created
    let init = json!({
        "type": "session_created",
        "session_id": initial_session,
    });
    if outbound_tx
        .send(OutboundFrame::Message(init))
        .await
        .is_err()
    {
        cleanup(&state, &chat_id).await;
        return;
    }

    // Track the current session_id per browser (session lives on the browser,
    // not in state — we only remember it here to echo back new session IDs).
    let mut current_session = initial_session;

    // Reader loop
    while let Some(Ok(msg)) = ws_stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "message" => {
                let content = parsed
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let media = parsed.get("media").cloned().unwrap_or(Value::Null);
                let session_id = parsed
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&current_session)
                    .to_string();
                forward_to_plexus(
                    &state,
                    &chat_id,
                    &user_id,
                    &content,
                    &media,
                    &session_id,
                    &outbound_tx,
                )
                .await;
            }
            "new_session" => {
                let new_id = format!("gateway:{}:{}", user_id, uuid::Uuid::new_v4());
                current_session = new_id.clone();
                let _ = outbound_tx
                    .send(OutboundFrame::Message(json!({
                        "type": "session_created",
                        "session_id": new_id,
                    })))
                    .await;
            }
            "switch_session" => {
                if let Some(sid) = parsed.get("session_id").and_then(|v| v.as_str()) {
                    current_session = sid.to_string();
                    let _ = outbound_tx
                        .send(OutboundFrame::Message(json!({
                            "type": "session_switched",
                            "session_id": sid,
                        })))
                        .await;
                }
            }
            other => {
                warn!("ws/chat: unknown message type: {other}");
            }
        }
    }

    cleanup(&state, &chat_id).await;
    let _ = writer.await;
    info!("ws/chat: browser disconnected chat_id={chat_id}");
}

async fn forward_to_plexus(
    state: &Arc<AppState>,
    chat_id: &str,
    user_id: &str,
    content: &str,
    media: &Value,
    session_id: &str,
    outbound_tx: &mpsc::Sender<OutboundFrame>,
) {
    let plexus_tx = {
        let guard = state.plexus.read().await;
        guard.clone()
    };
    let Some(plexus_tx) = plexus_tx else {
        let _ = outbound_tx
            .send(OutboundFrame::Message(json!({
                "type": "error",
                "reason": "Plexus server not connected",
            })))
            .await;
        return;
    };
    let mut payload = json!({
        "type": "message",
        "chat_id": chat_id,
        "sender_id": user_id,
        "content": content,
        "session_id": session_id,
    });
    if !media.is_null() {
        payload["media"] = media.clone();
    }
    if plexus_tx.send(payload).await.is_err() {
        warn!("ws/chat: plexus channel closed while forwarding");
    }
}

async fn writer_task(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<OutboundFrame>,
) {
    while let Some(frame) = rx.recv().await {
        let value = match frame {
            OutboundFrame::Message(v) | OutboundFrame::Progress(v) => v,
        };
        let text = serde_json::to_string(&value).unwrap_or_default();
        if sink.send(Message::Text(text.into())).await.is_err() {
            break;
        }
    }
}

async fn cleanup(state: &Arc<AppState>, chat_id: &str) {
    state.browsers.remove(chat_id);
}
```

- [ ] **Step 2: Wire `/ws/chat` into the router in `main.rs`**

Update `plexus-gateway/src/main.rs`:

```rust
mod config;
mod jwt;
mod routing;
mod state;
mod static_files;
mod ws;

use axum::Router;
use axum::routing::get;
use config::Config;
use state::AppState;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    let state = AppState::new(config.clone());

    let app: Router = Router::new()
        .route("/ws/chat", get(ws::chat::handler))
        .merge(static_files::service(&config.frontend_dir))
        .with_state(Arc::clone(&state));

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("plexus-gateway listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 3: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build.

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/ws/chat.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add /ws/chat browser WebSocket handler"
```

---

### Task 8: Plexus-Server WebSocket Handler (`/ws/plexus`)

**Files:**
- Replace: `plexus-gateway/src/ws/plexus.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Implement plexus handler**

Replace `plexus-gateway/src/ws/plexus.rs`:

```rust
//! /ws/plexus plexus-server WebSocket handler.
//!
//! Flow:
//! 1. Upgrade without query auth.
//! 2. Wait up to 5s for the first text frame — must be {"type":"auth","token":...}.
//! 3. Compare token constant-time against PLEXUS_GATEWAY_TOKEN.
//! 4. Reject duplicate connections with auth_fail.
//! 5. Run a writer task owning the sink; run a reader loop routing `send` messages.

use crate::routing;
use crate::state::AppState;
use axum::extract::{
    State, WebSocketUpgrade,
    ws::{Message, WebSocket},
};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tracing::{info, warn};

pub async fn handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| run(socket, state))
}

async fn run(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Step 1: wait for auth frame (5s)
    let auth_msg = match timeout(Duration::from_secs(5), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => t,
        _ => {
            let _ = sink
                .send(Message::Text(
                    json!({"type":"auth_fail","reason":"no auth"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    let parsed: Value = match serde_json::from_str(&auth_msg) {
        Ok(v) => v,
        Err(_) => {
            let _ = sink
                .send(Message::Text(
                    json!({"type":"auth_fail","reason":"invalid json"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    if parsed.get("type").and_then(|v| v.as_str()) != Some("auth") {
        let _ = sink
            .send(Message::Text(
                json!({"type":"auth_fail","reason":"expected auth"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    }
    let provided = parsed
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .as_bytes();
    let expected = state.config.gateway_token.as_bytes();
    let ok = provided.len() == expected.len() && provided.ct_eq(expected).into();
    if !ok {
        let _ = sink
            .send(Message::Text(
                json!({"type":"auth_fail","reason":"invalid token"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    }

    // Step 2: enforce singleton
    let (plexus_tx, mut plexus_rx) = mpsc::channel::<Value>(256);
    {
        let mut guard = state.plexus.write().await;
        if guard.is_some() {
            let _ = sink
                .send(Message::Text(
                    json!({"type":"auth_fail","reason":"duplicate connection"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
        *guard = Some(plexus_tx);
    }

    // Step 3: ack
    if sink
        .send(Message::Text(
            json!({"type":"auth_ok"}).to_string().into(),
        ))
        .await
        .is_err()
    {
        state.plexus.write().await.take();
        return;
    }
    info!("ws/plexus: server authenticated");

    // Writer task: drain plexus_rx into the sink
    let writer = tokio::spawn(async move {
        while let Some(value) = plexus_rx.recv().await {
            let text = serde_json::to_string(&value).unwrap_or_default();
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Reader loop: route `send` messages
    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if msg_type != "send" {
            warn!("ws/plexus: unknown message type: {msg_type}");
            continue;
        }
        let _ = routing::route_send(&state, &parsed).await;
    }

    // Cleanup
    state.plexus.write().await.take();
    let _ = writer.await;
    info!("ws/plexus: server disconnected");
}
```

- [ ] **Step 2: Wire `/ws/plexus` into the router in `main.rs`**

Add the route in `plexus-gateway/src/main.rs`:

```rust
    let app: Router = Router::new()
        .route("/ws/chat", get(ws::chat::handler))
        .route("/ws/plexus", get(ws::plexus::handler))
        .merge(static_files::service(&config.frontend_dir))
        .with_state(Arc::clone(&state));
```

- [ ] **Step 3: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build.

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/ws/plexus.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add /ws/plexus server WebSocket handler"
```

---

### Task 9: REST Proxy

**Files:**
- Create: `plexus-gateway/src/proxy.rs`
- Modify: `plexus-gateway/src/main.rs`

- [ ] **Step 1: Create `proxy.rs`**

```rust
//! REST reverse proxy for /api/*.
//!
//! - Public endpoints (/api/auth/login, /api/auth/register) skip JWT.
//! - All other paths require a valid JWT at the gateway before proxying.
//! - Max body: 25 MB (enforced by tower-http RequestBodyLimitLayer).
//! - Hop-by-hop headers stripped.
//! - Path traversal rejected.

use crate::jwt;
use crate::state::AppState;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::json;
use std::sync::Arc;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::warn;

const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

const HOP_BY_HOP: &[&str] = &[
    "host",
    "connection",
    "transfer-encoding",
    "upgrade",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
];

fn is_public(path: &str) -> bool {
    matches!(path, "/api/auth/login" | "/api/auth/register")
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/{*rest}", any(handler))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
}

async fn handler(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());

    // Path traversal block
    if path.contains("..") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            error_json("path traversal rejected"),
        )
            .into_response();
    }

    // Auth gate
    if !is_public(&path) {
        let headers = req.headers();
        let token = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        let ok = token
            .and_then(|t| jwt::validate(t, &state.config.jwt_secret).ok())
            .is_some();
        if !ok {
            return (StatusCode::UNAUTHORIZED, error_json("invalid or missing token"))
                .into_response();
        }
    }

    // Build upstream URL
    let upstream_url = {
        let base = state.config.plexus_server_api_url.trim_end_matches('/');
        match query {
            Some(q) => format!("{base}{path}?{q}"),
            None => format!("{base}{path}"),
        }
    };

    let method = req.method().clone();
    let headers = req.headers().clone();
    let body_bytes = match to_bytes(req.into_body(), MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, error_json("body too large"))
                .into_response();
        }
    };

    // Build reqwest request
    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, error_json("invalid method"))
                .into_response();
        }
    };

    let mut upstream_headers = reqwest::header::HeaderMap::new();
    for (name, value) in headers.iter() {
        let n = name.as_str().to_ascii_lowercase();
        if HOP_BY_HOP.contains(&n.as_str()) {
            continue;
        }
        if let (Ok(hn), Ok(hv)) = (
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            upstream_headers.insert(hn, hv);
        }
    }

    let upstream = state
        .http_client
        .request(reqwest_method, &upstream_url)
        .headers(upstream_headers)
        .body(body_bytes.to_vec())
        .send()
        .await;

    let resp = match upstream {
        Ok(r) => r,
        Err(e) => {
            warn!("proxy: upstream error: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                error_json(&format!("upstream unreachable: {e}")),
            )
                .into_response();
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut out_headers = HeaderMap::new();
    for (name, value) in resp.headers().iter() {
        let n = name.as_str().to_ascii_lowercase();
        if HOP_BY_HOP.contains(&n.as_str()) {
            continue;
        }
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            out_headers.insert(hn, hv);
        }
    }
    let body = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                error_json(&format!("read upstream body: {e}")),
            )
                .into_response();
        }
    };

    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = out_headers;
    response
}

fn error_json(msg: &str) -> String {
    json!({
        "error": {
            "code": "gateway_error",
            "message": msg,
        }
    })
    .to_string()
}

// Silence unused imports if the compiler folds handler signature types.
#[allow(dead_code)]
fn _unused(_: Uri, _: Method) {}
```

- [ ] **Step 2: Wire proxy into the router in `main.rs`**

Add proxy routes in `plexus-gateway/src/main.rs`:

```rust
mod config;
mod jwt;
mod proxy;
mod routing;
mod state;
mod static_files;
mod ws;

use axum::Router;
use axum::routing::get;
use config::Config;
use state::AppState;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    let state = AppState::new(config.clone());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app: Router = Router::new()
        .route("/ws/chat", get(ws::chat::handler))
        .route("/ws/plexus", get(ws::plexus::handler))
        .merge(proxy::routes())
        .merge(static_files::service(&config.frontend_dir))
        .layer(cors)
        .with_state(Arc::clone(&state));

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("plexus-gateway listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 3: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build (may warn about unused imports in proxy.rs — those are fine).

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/proxy.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add REST reverse proxy for /api/*"
```

---

### Task 10: Integration Tests + Postman Validation Gate

**Files:**
- Create: `plexus-gateway/tests/integration.rs`

- [ ] **Step 1: Write end-to-end integration tests**

Create `plexus-gateway/tests/integration.rs`:

```rust
//! End-to-end tests: spin up the gateway on an ephemeral port, connect a
//! mock browser (WS) and mock plexus (WS), assert messages flow.
//!
//! Each test builds its own isolated Config (no env vars) and binds a fresh
//! listener, so tests run fine in parallel.

use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{EncodingKey, Header, encode};
use plexus_gateway::config::Config;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

const JWT_SECRET: &str = "test-jwt-secret";
const GATEWAY_TOKEN: &str = "test-gateway-token";

#[derive(Serialize)]
struct Claims {
    sub: String,
    is_admin: bool,
    exp: i64,
}

fn valid_jwt(user_id: &str) -> String {
    let c = Claims {
        sub: user_id.into(),
        is_admin: false,
        exp: chrono::Utc::now().timestamp() + 3600,
    };
    encode(
        &Header::default(),
        &c,
        &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )
    .unwrap()
}

/// Start a gateway instance in-process on an ephemeral port. Returns the port.
async fn spawn_gateway() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let config = Config {
        gateway_token: GATEWAY_TOKEN.to_string(),
        jwt_secret: JWT_SECRET.to_string(),
        port,
        plexus_server_api_url: "http://127.0.0.1:1".to_string(),
        frontend_dir: "/tmp".to_string(),
    };

    tokio::spawn(async move {
        plexus_gateway::serve(listener, config).await;
    });

    // Wait briefly for the server to be ready by connecting once.
    for _ in 0..20 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return port;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("gateway failed to bind within ~500ms");
}

async fn connect_plexus(port: u16) -> tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
> {
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    // Auth
    ws.send(WsMessage::Text(
        json!({"type":"auth","token": GATEWAY_TOKEN})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "auth_ok");
    ws
}

async fn connect_browser(
    port: u16,
    user: &str,
) -> tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
> {
    let jwt = valid_jwt(user);
    let (mut ws, _) = connect_async(format!(
        "ws://127.0.0.1:{port}/ws/chat?token={jwt}"
    ))
    .await
    .unwrap();
    // Expect session_created
    let msg = ws.next().await.unwrap().unwrap();
    let text = match msg {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "session_created");
    ws
}

#[tokio::test]
async fn browser_to_plexus_round_trip() {
    let port = spawn_gateway().await;
    let mut plexus = connect_plexus(port).await;
    let mut browser = connect_browser(port, "user-1").await;

    // Browser sends a message
    browser
        .send(WsMessage::Text(
            json!({"type":"message","content":"hello"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    // Plexus receives it
    let msg = plexus.next().await.unwrap().unwrap();
    let text = match msg {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "hello");
    assert_eq!(v["sender_id"], "user-1");
    assert!(v["chat_id"].is_string());
}

#[tokio::test]
async fn plexus_send_reaches_browser() {
    let port = spawn_gateway().await;
    let mut plexus = connect_plexus(port).await;
    let mut browser = connect_browser(port, "user-2").await;

    // Browser sends first to establish routing
    browser
        .send(WsMessage::Text(
            json!({"type":"message","content":"ping"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    // Plexus reads the message, grabs chat_id, sends back
    let incoming = plexus.next().await.unwrap().unwrap();
    let text = match incoming {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    let chat_id = v["chat_id"].as_str().unwrap().to_string();

    plexus
        .send(WsMessage::Text(
            json!({
                "type": "send",
                "chat_id": chat_id,
                "content": "pong",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // Browser receives it
    let reply = browser.next().await.unwrap().unwrap();
    let text = match reply {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "pong");
}

#[tokio::test]
async fn browser_without_plexus_gets_error() {
    let port = spawn_gateway().await;
    let mut browser = connect_browser(port, "user-3").await;

    browser
        .send(WsMessage::Text(
            json!({"type":"message","content":"lonely"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let reply = browser.next().await.unwrap().unwrap();
    let text = match reply {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "error");
    assert!(v["reason"].as_str().unwrap().contains("not connected"));
}

#[tokio::test]
async fn invalid_jwt_rejected() {
    let port = spawn_gateway().await;
    let res = connect_async(format!(
        "ws://127.0.0.1:{port}/ws/chat?token=not-a-valid-jwt"
    ))
    .await;
    assert!(res.is_err(), "expected upgrade rejection");
}

#[tokio::test]
async fn duplicate_plexus_rejected() {
    let port = spawn_gateway().await;
    let _p1 = connect_plexus(port).await;
    // Second connection should get auth_fail
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/plexus"))
        .await
        .unwrap();
    ws.send(WsMessage::Text(
        json!({"type":"auth","token": GATEWAY_TOKEN})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    let text = match msg {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "auth_fail");
}
```

- [ ] **Step 2: Create `lib.rs` helper for tests**

Integration tests need to import from the gateway crate. Tests also need to avoid env-var races, so the library exposes two entry points: one that reads env vars (for `main.rs`) and one that accepts a `Config` directly (for tests).

Create `plexus-gateway/src/lib.rs`:

```rust
//! Library entry point for integration tests.

pub mod config;
pub mod jwt;
pub mod proxy;
pub mod routing;
pub mod state;
pub mod static_files;
pub mod ws;

use axum::Router;
use axum::routing::get;
use config::Config;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

/// Build the axum router. Exposed for tests.
pub fn build_router(state: Arc<state::AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/ws/chat", get(ws::chat::handler))
        .route("/ws/plexus", get(ws::plexus::handler))
        .merge(proxy::routes())
        .merge(static_files::service(&state.config.frontend_dir))
        .layer(cors)
        .with_state(state)
}

/// Run the gateway on a TCP listener with the given config. Used by tests
/// that want to spawn isolated instances without touching env vars.
pub async fn serve(listener: tokio::net::TcpListener, config: Config) {
    let state = state::AppState::new(config);
    let app = build_router(state);
    axum::serve(listener, app).await.unwrap();
}

/// Run the gateway using env vars for config. Used by `main.rs`.
pub async fn run_from_env() {
    let config = Config::from_env();
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("plexus-gateway listening on {addr}");
    serve(listener, config).await;
}
```

- [ ] **Step 3: Trim `main.rs` to delegate to `lib.rs`**

Replace `plexus-gateway/src/main.rs`:

```rust
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    plexus_gateway::run_from_env().await;
}
```

- [ ] **Step 4: Add `chrono` to `[dev-dependencies]` if not already there**

Already in `[dependencies]`, so nothing to do. Verify the Cargo.toml snippet from Task 1 includes `chrono`.

- [ ] **Step 5: Run tests**

Run: `cargo test --package plexus-gateway`
Expected: All 5 integration tests + all unit tests pass.

- [ ] **Step 6: Commit**

```bash
git add plexus-gateway/src/lib.rs plexus-gateway/src/main.rs plexus-gateway/tests/
git commit -m "test(gateway): add end-to-end integration tests"
```

- [ ] **Step 7: USER VALIDATION GATE**

This is a hard gate. Do not start Phase 2 until the user has validated Phase 1 with Postman.

Launch the gateway alongside a running plexus-server:

```bash
# Terminal 1
cargo run --package plexus-server

# Terminal 2
cargo run --package plexus-gateway
```

Ask the user to validate with Postman:

1. **Register/Login through the gateway:** `POST http://localhost:9090/api/auth/register` and `POST http://localhost:9090/api/auth/login`. Both should return a JWT.
2. **Protected endpoint:** `GET http://localhost:9090/api/user/profile` with `Authorization: Bearer <JWT>`. Should return the user.
3. **Browser WebSocket:** Connect to `ws://localhost:9090/ws/chat?token=<JWT>`. Expect `session_created`. Send `{"type":"message","content":"hello"}`. Plexus-server should receive it and the browser should see the agent's reply.
4. **Error flow:** Kill plexus-server. Send a browser message. Expect `{"type":"error","reason":"Plexus server not connected"}`.
5. **Static files:** Create a dummy `plexus-frontend/dist/index.html` and verify `GET http://localhost:9090/` returns it.

Wait for the user to confirm "Phase 1 validated" before beginning Task 11.

---

## Phase 2 — plexus-frontend

### Task 11: Frontend Project Scaffold

**Files:**
- Create: `plexus-frontend/package.json`
- Create: `plexus-frontend/tsconfig.json`
- Create: `plexus-frontend/tsconfig.node.json`
- Create: `plexus-frontend/vite.config.ts`
- Create: `plexus-frontend/tailwind.config.ts`
- Create: `plexus-frontend/postcss.config.js`
- Create: `plexus-frontend/index.html`
- Create: `plexus-frontend/src/main.tsx`
- Create: `plexus-frontend/src/styles/globals.css`
- Create: `plexus-frontend/.gitignore`

- [ ] **Step 1: Create `package.json`**

```json
{
  "name": "plexus-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "test:watch": "vitest"
  },
  "dependencies": {
    "lucide-react": "^0.462.0",
    "react": "^19.2.4",
    "react-dom": "^19.2.4",
    "react-markdown": "^10.0.0",
    "react-router-dom": "^7.0.0",
    "react-syntax-highlighter": "^16.0.0",
    "remark-gfm": "^4.0.0",
    "zustand": "^5.0.0"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.0.0",
    "@testing-library/jest-dom": "^6.6.0",
    "@testing-library/react": "^16.1.0",
    "@types/node": "^22.10.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@types/react-syntax-highlighter": "^15.5.13",
    "@vitejs/plugin-react": "^5.0.0",
    "jsdom": "^26.0.0",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.9.0",
    "vite": "^8.0.0",
    "vitest": "^2.1.0"
  }
}
```

- [ ] **Step 2: Create `tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "baseUrl": ".",
    "paths": {
      "@/*": ["src/*"]
    },
    "types": ["vitest/globals", "@testing-library/jest-dom"]
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

- [ ] **Step 3: Create `tsconfig.node.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "noEmit": true,
    "types": ["node"]
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **Step 4: Create `vite.config.ts`**

```ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://localhost:9090',
      '/ws': { target: 'ws://localhost:9090', ws: true, changeOrigin: true },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: './src/test-setup.ts',
  },
})
```

- [ ] **Step 5: Create `tailwind.config.ts`** (theme tokens live here for IDE hinting, Tailwind v4 also picks them up from CSS)

```ts
import type { Config } from 'tailwindcss'

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        plex: {
          bg: '#0d1117',
          sidebar: '#0a0f18',
          card: '#161b22',
          border: '#1a2332',
          accent: '#39ff14',
          muted: '#8b949e',
          text: '#e6edf3',
          danger: '#ff4444',
        },
      },
    },
  },
} satisfies Config
```

- [ ] **Step 6: Create `index.html`**

```html
<!doctype html>
<html lang="en" class="dark">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Plexus</title>
  </head>
  <body class="bg-plex-bg text-plex-text antialiased">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 7: Create `src/styles/globals.css`**

```css
@import "tailwindcss";

@theme {
  --color-plex-bg: #0d1117;
  --color-plex-sidebar: #0a0f18;
  --color-plex-card: #161b22;
  --color-plex-border: #1a2332;
  --color-plex-accent: #39ff14;
  --color-plex-muted: #8b949e;
  --color-plex-text: #e6edf3;
  --color-plex-danger: #ff4444;
}

html, body, #root { height: 100%; }

body {
  font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI',
    'Helvetica Neue', Arial, sans-serif;
}

code, pre, .mono {
  font-family: 'JetBrains Mono', 'Fira Code', Menlo, Consolas, monospace;
}

/* Thin scrollbar */
::-webkit-scrollbar { width: 8px; height: 8px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: #1a2332; border-radius: 4px; }
::-webkit-scrollbar-thumb:hover { background: #2a3342; }
```

- [ ] **Step 8: Create `src/main.tsx` stub**

```tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import './styles/globals.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <div className="p-8 text-plex-accent font-mono">
      plexus-frontend scaffolded
    </div>
  </React.StrictMode>,
)
```

- [ ] **Step 9: Create `src/test-setup.ts`**

```ts
import '@testing-library/jest-dom'
```

- [ ] **Step 10: Create `plexus-frontend/.gitignore`**

```
node_modules
dist
dist-ssr
*.local
.vite
coverage
```

- [ ] **Step 11: Install and run**

```bash
cd plexus-frontend
npm install
npm run build
npm run dev &
sleep 3
curl -s http://localhost:5173/ | grep -q 'id="root"' && echo "dev OK"
kill %1
```

Expected: `dist/` is created, dev server returns HTML with `id="root"`.

- [ ] **Step 12: Commit**

```bash
git add plexus-frontend/
git commit -m "feat(frontend): scaffold Vite + React + Tailwind + Vitest"
```

---

### Task 12: TypeScript Types + API Wrapper

**Files:**
- Create: `plexus-frontend/src/lib/types.ts`
- Create: `plexus-frontend/src/lib/api.ts`

- [ ] **Step 1: Create `types.ts`**

```ts
// Mirrors plexus-server API responses. Update if API.md changes.

export interface UserProfile {
  user_id: string
  email: string
  is_admin: boolean
  created_at: string
}

export interface AuthResponse {
  token: string
  user_id: string
  is_admin: boolean
}

export interface Session {
  session_id: string
  created_at: string
}

export type MessageRole = 'user' | 'assistant' | 'tool' | 'system'

export interface DbMessage {
  message_id: string
  role: MessageRole
  content: string
  tool_call_id: string | null
  tool_name: string | null
  tool_arguments: string | null
  created_at: string
}

export interface Device {
  device_name: string
  status: 'online' | 'offline'
  last_seen_secs_ago: number
  tools_count: number
  fs_policy: { mode: 'sandbox' | 'unrestricted' }
}

export interface DeviceToken {
  token: string
  device_name: string
  created_at: string
}

export interface McpServerEntry {
  name: string
  command: string
  args: string[]
}

export interface Skill {
  skill_id: string
  name: string
  description: string
  always_on: boolean
  skill_path: string
}

export interface CronJob {
  job_id: string
  name: string | null
  enabled: boolean
  cron_expr: string | null
  every_seconds: number | null
  timezone: string
  message: string
  channel: string
  chat_id: string
  delete_after_run: boolean
  next_run_at: string | null
  last_run_at: string | null
  run_count: number
}

export interface DiscordConfig {
  user_id: string
  bot_user_id?: string
  enabled: boolean
  allowed_users: string[]
  owner_discord_id?: string
}

export interface TelegramConfig {
  user_id: string
  enabled: boolean
  partner_telegram_id: string
  allowed_users: string[]
  group_policy: 'mention' | 'all'
}

export interface LlmConfig {
  api_base: string
  model: string
  api_key: string
  context_window: number
}

// WebSocket message shapes (browser ↔ gateway)
export type WsIncoming =
  | { type: 'session_created'; session_id: string }
  | { type: 'session_switched'; session_id: string }
  | { type: 'message'; content: string; session_id: string; media?: string[] }
  | { type: 'progress'; content: string; session_id: string }
  | { type: 'error'; reason: string }

export type WsOutgoing =
  | { type: 'message'; content: string; media?: string[] }
  | { type: 'new_session' }
  | { type: 'switch_session'; session_id: string }
```

- [ ] **Step 2: Create `api.ts`**

```ts
// Fetch wrapper that attaches the JWT and handles 401 uniformly.

import { useAuthStore } from '@/store/auth'

export class ApiError extends Error {
  status: number
  code: string
  constructor(status: number, code: string, message: string) {
    super(message)
    this.status = status
    this.code = code
  }
}

interface Options extends RequestInit {
  json?: unknown
}

export async function api<T = unknown>(
  path: string,
  opts: Options = {},
): Promise<T> {
  const headers = new Headers(opts.headers)
  const token = useAuthStore.getState().token
  if (token) headers.set('Authorization', `Bearer ${token}`)
  if (opts.json !== undefined) {
    headers.set('Content-Type', 'application/json')
    opts.body = JSON.stringify(opts.json)
  }
  const res = await fetch(path, { ...opts, headers })
  if (res.status === 401) {
    useAuthStore.getState().logout()
    throw new ApiError(401, 'unauthorized', 'Session expired')
  }
  if (!res.ok) {
    let code = 'http_error'
    let message = res.statusText
    try {
      const body = await res.json()
      if (body?.error?.code) code = body.error.code
      if (body?.error?.message) message = body.error.message
    } catch {
      // ignore
    }
    throw new ApiError(res.status, code, message)
  }
  if (res.status === 204) return undefined as T
  const ctype = res.headers.get('content-type') ?? ''
  if (ctype.includes('application/json')) return (await res.json()) as T
  return (await res.text()) as unknown as T
}
```

- [ ] **Step 3: Build to type-check**

Run: `cd plexus-frontend && npm run build`
Expected: Errors! `api.ts` imports from `@/store/auth` which doesn't exist yet. That's intentional — the next task creates it. Skip the build for now.

Instead run a looser check:

```bash
cd plexus-frontend && npx tsc --noEmit --skipLibCheck src/lib/types.ts
```

Expected: Clean.

- [ ] **Step 4: Commit**

```bash
git add plexus-frontend/src/lib/
git commit -m "feat(frontend): add API types and fetch wrapper"
```

---

### Task 13: Auth Store + Login Page + Router Skeleton

**Files:**
- Create: `plexus-frontend/src/store/auth.ts`
- Create: `plexus-frontend/src/pages/Login.tsx`
- Create: `plexus-frontend/src/pages/Chat.tsx` (stub)
- Create: `plexus-frontend/src/pages/Settings.tsx` (stub)
- Create: `plexus-frontend/src/pages/Admin.tsx` (stub)
- Create: `plexus-frontend/src/App.tsx`
- Modify: `plexus-frontend/src/main.tsx`

- [ ] **Step 1: Create `store/auth.ts`**

```ts
import { create } from 'zustand'
import { api } from '@/lib/api'
import type { AuthResponse } from '@/lib/types'

interface AuthState {
  token: string | null
  userId: string | null
  isAdmin: boolean
  login: (email: string, password: string) => Promise<void>
  logout: () => void
}

const STORAGE_KEY = 'plexus.jwt'

function loadInitial(): Pick<AuthState, 'token' | 'userId' | 'isAdmin'> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { token: null, userId: null, isAdmin: false }
    const parsed = JSON.parse(raw) as { token: string; userId: string; isAdmin: boolean }
    return { token: parsed.token, userId: parsed.userId, isAdmin: parsed.isAdmin }
  } catch {
    return { token: null, userId: null, isAdmin: false }
  }
}

export const useAuthStore = create<AuthState>((set) => ({
  ...loadInitial(),
  async login(email, password) {
    const res = await api<AuthResponse>('/api/auth/login', {
      method: 'POST',
      json: { email, password },
    })
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ token: res.token, userId: res.user_id, isAdmin: res.is_admin }),
    )
    set({ token: res.token, userId: res.user_id, isAdmin: res.is_admin })
  },
  logout() {
    localStorage.removeItem(STORAGE_KEY)
    set({ token: null, userId: null, isAdmin: false })
  },
}))
```

- [ ] **Step 2: Create `pages/Login.tsx`**

```tsx
import { FormEvent, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/store/auth'

export default function Login() {
  const login = useAuthStore((s) => s.login)
  const nav = useNavigate()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  async function onSubmit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    setLoading(true)
    try {
      await login(email, password)
      nav('/chat')
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex h-full items-center justify-center bg-plex-bg">
      <form
        onSubmit={onSubmit}
        className="w-full max-w-sm rounded-lg border border-plex-border bg-plex-card p-8"
      >
        <h1 className="mb-6 text-2xl font-semibold text-plex-accent">Plexus</h1>
        <label className="mb-4 block">
          <span className="text-xs uppercase tracking-wide text-plex-muted">Email</span>
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            className="mt-1 block w-full rounded border border-plex-border bg-plex-sidebar px-3 py-2 text-plex-text focus:border-plex-accent focus:outline-none"
          />
        </label>
        <label className="mb-6 block">
          <span className="text-xs uppercase tracking-wide text-plex-muted">Password</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            className="mt-1 block w-full rounded border border-plex-border bg-plex-sidebar px-3 py-2 text-plex-text focus:border-plex-accent focus:outline-none"
          />
        </label>
        {error && (
          <p className="mb-4 text-sm text-plex-danger">{error}</p>
        )}
        <button
          type="submit"
          disabled={loading}
          className="w-full rounded bg-plex-accent py-2 font-semibold text-black hover:brightness-110 disabled:opacity-50"
        >
          {loading ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </div>
  )
}
```

- [ ] **Step 3: Create stub pages**

`plexus-frontend/src/pages/Chat.tsx`:

```tsx
export default function Chat() {
  return <div className="p-8 text-plex-text">Chat page (stub)</div>
}
```

`plexus-frontend/src/pages/Settings.tsx`:

```tsx
export default function Settings() {
  return <div className="p-8 text-plex-text">Settings page (stub)</div>
}
```

`plexus-frontend/src/pages/Admin.tsx`:

```tsx
export default function Admin() {
  return <div className="p-8 text-plex-text">Admin page (stub)</div>
}
```

- [ ] **Step 4: Create `App.tsx`**

```tsx
import { Navigate, Route, Routes } from 'react-router-dom'
import { useAuthStore } from '@/store/auth'
import Login from '@/pages/Login'
import Chat from '@/pages/Chat'
import Settings from '@/pages/Settings'
import Admin from '@/pages/Admin'

function Protected({ children }: { children: React.ReactNode }) {
  const token = useAuthStore((s) => s.token)
  if (!token) return <Navigate to="/login" replace />
  return <>{children}</>
}

function AdminOnly({ children }: { children: React.ReactNode }) {
  const token = useAuthStore((s) => s.token)
  const isAdmin = useAuthStore((s) => s.isAdmin)
  if (!token) return <Navigate to="/login" replace />
  if (!isAdmin) return <Navigate to="/chat" replace />
  return <>{children}</>
}

export default function App() {
  return (
    <Routes>
      <Route path="/login" element={<Login />} />
      <Route path="/" element={<Navigate to="/chat" replace />} />
      <Route
        path="/chat"
        element={
          <Protected>
            <Chat />
          </Protected>
        }
      />
      <Route
        path="/chat/:sessionId"
        element={
          <Protected>
            <Chat />
          </Protected>
        }
      />
      <Route
        path="/settings"
        element={
          <Protected>
            <Settings />
          </Protected>
        }
      />
      <Route
        path="/admin"
        element={
          <AdminOnly>
            <Admin />
          </AdminOnly>
        }
      />
      <Route path="*" element={<Navigate to="/chat" replace />} />
    </Routes>
  )
}
```

- [ ] **Step 5: Update `main.tsx`**

Replace `plexus-frontend/src/main.tsx`:

```tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'
import App from './App'
import './styles/globals.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
)
```

- [ ] **Step 6: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 7: Commit**

```bash
git add plexus-frontend/src/
git commit -m "feat(frontend): auth store, login page, and router skeleton"
```

---

### Task 14: WebSocket Manager

**Files:**
- Create: `plexus-frontend/src/lib/ws.ts`

- [ ] **Step 1: Create `ws.ts`**

```ts
// Singleton WebSocket manager with auto-reconnect.

import type { WsIncoming, WsOutgoing } from '@/lib/types'

type Listener = (msg: WsIncoming) => void
type StatusListener = (status: 'connecting' | 'open' | 'closed') => void

class WebSocketManager {
  private ws: WebSocket | null = null
  private listeners = new Set<Listener>()
  private statusListeners = new Set<StatusListener>()
  private token: string | null = null
  private reconnectAttempts = 0
  private reconnectTimer: number | null = null
  private shouldReconnect = false

  connect(token: string) {
    if (this.ws && this.token === token) return
    this.token = token
    this.shouldReconnect = true
    this.dial()
  }

  disconnect() {
    this.shouldReconnect = false
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    if (this.ws) {
      this.ws.close()
      this.ws = null
    }
    this.setStatus('closed')
  }

  send(msg: WsOutgoing) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      console.warn('ws: send while not open', msg)
      return
    }
    this.ws.send(JSON.stringify(msg))
  }

  onMessage(l: Listener): () => void {
    this.listeners.add(l)
    return () => this.listeners.delete(l)
  }

  onStatus(l: StatusListener): () => void {
    this.statusListeners.add(l)
    return () => this.statusListeners.delete(l)
  }

  private dial() {
    if (!this.token) return
    this.setStatus('connecting')
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const url = `${protocol}//${window.location.host}/ws/chat?token=${encodeURIComponent(this.token)}`
    const ws = new WebSocket(url)
    this.ws = ws

    ws.onopen = () => {
      this.reconnectAttempts = 0
      this.setStatus('open')
    }
    ws.onmessage = (e) => {
      try {
        const msg = JSON.parse(e.data) as WsIncoming
        this.listeners.forEach((l) => l(msg))
      } catch (err) {
        console.warn('ws: invalid message', err)
      }
    }
    ws.onclose = () => {
      this.ws = null
      this.setStatus('closed')
      if (this.shouldReconnect) this.scheduleReconnect()
    }
    ws.onerror = (e) => {
      console.warn('ws: error', e)
    }
  }

  private scheduleReconnect() {
    if (this.reconnectTimer !== null) return
    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30_000)
    this.reconnectAttempts++
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null
      this.dial()
    }, delay)
  }

  private setStatus(status: 'connecting' | 'open' | 'closed') {
    this.statusListeners.forEach((l) => l(status))
  }
}

export const wsManager = new WebSocketManager()
```

- [ ] **Step 2: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add plexus-frontend/src/lib/ws.ts
git commit -m "feat(frontend): singleton WebSocket manager with reconnect"
```

---

### Task 15: Chat Store

**Files:**
- Create: `plexus-frontend/src/store/chat.ts`

- [ ] **Step 1: Create `chat.ts`**

```ts
import { create } from 'zustand'
import { api } from '@/lib/api'
import { wsManager } from '@/lib/ws'
import type { DbMessage, Session, WsIncoming, WsOutgoing } from '@/lib/types'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant' | 'tool' | 'system'
  content: string
  media?: string[]
  created_at: string
}

interface ChatState {
  sessions: Session[]
  currentSessionId: string | null
  messagesBySession: Record<string, ChatMessage[]>
  progressBySession: Record<string, string | null>
  wsStatus: 'connecting' | 'open' | 'closed'
  init: () => void
  loadSessions: () => Promise<void>
  loadMessages: (sessionId: string) => Promise<void>
  setCurrentSession: (sessionId: string) => void
  newSession: () => void
  sendMessage: (content: string, media?: string[]) => void
  handleIncoming: (msg: WsIncoming) => void
}

export const useChatStore = create<ChatState>((set, get) => ({
  sessions: [],
  currentSessionId: null,
  messagesBySession: {},
  progressBySession: {},
  wsStatus: 'closed',

  init() {
    wsManager.onMessage((m) => get().handleIncoming(m))
    wsManager.onStatus((s) => set({ wsStatus: s }))
  },

  async loadSessions() {
    const sessions = await api<Session[]>('/api/sessions')
    set({ sessions })
  },

  async loadMessages(sessionId) {
    const rows = await api<DbMessage[]>(
      `/api/sessions/${encodeURIComponent(sessionId)}/messages?limit=200`,
    )
    const messages: ChatMessage[] = rows
      .filter((r) => r.role === 'user' || r.role === 'assistant')
      .map((r) => ({
        id: r.message_id,
        role: r.role as ChatMessage['role'],
        content: r.content,
        created_at: r.created_at,
      }))
    set((s) => ({
      messagesBySession: { ...s.messagesBySession, [sessionId]: messages },
    }))
  },

  setCurrentSession(sessionId) {
    set((s) => ({
      currentSessionId: sessionId,
      progressBySession: { ...s.progressBySession, [sessionId]: null },
    }))
    const msg: WsOutgoing = { type: 'switch_session', session_id: sessionId }
    wsManager.send(msg)
  },

  newSession() {
    const msg: WsOutgoing = { type: 'new_session' }
    wsManager.send(msg)
  },

  sendMessage(content, media) {
    const sessionId = get().currentSessionId
    if (!sessionId) return
    const local: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content,
      media,
      created_at: new Date().toISOString(),
    }
    set((s) => ({
      messagesBySession: {
        ...s.messagesBySession,
        [sessionId]: [...(s.messagesBySession[sessionId] ?? []), local],
      },
    }))
    const msg: WsOutgoing = { type: 'message', content, media }
    wsManager.send(msg)
  },

  handleIncoming(msg) {
    switch (msg.type) {
      case 'session_created':
      case 'session_switched': {
        set((s) => ({
          currentSessionId: msg.session_id,
          sessions: s.sessions.some((x) => x.session_id === msg.session_id)
            ? s.sessions
            : [{ session_id: msg.session_id, created_at: new Date().toISOString() }, ...s.sessions],
          progressBySession: { ...s.progressBySession, [msg.session_id]: null },
        }))
        break
      }
      case 'message': {
        const sid = msg.session_id
        const entry: ChatMessage = {
          id: crypto.randomUUID(),
          role: 'assistant',
          content: msg.content,
          media: msg.media,
          created_at: new Date().toISOString(),
        }
        set((s) => ({
          messagesBySession: {
            ...s.messagesBySession,
            [sid]: [...(s.messagesBySession[sid] ?? []), entry],
          },
          progressBySession: { ...s.progressBySession, [sid]: null },
        }))
        break
      }
      case 'progress': {
        const sid = msg.session_id
        set((s) => ({
          progressBySession: { ...s.progressBySession, [sid]: msg.content },
        }))
        break
      }
      case 'error': {
        console.error('ws error:', msg.reason)
        break
      }
    }
  },
}))
```

- [ ] **Step 2: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add plexus-frontend/src/store/chat.ts
git commit -m "feat(frontend): chat store with messages and progress hints"
```

---

### Task 16: Devices Store + Markdown Component

**Files:**
- Create: `plexus-frontend/src/store/devices.ts`
- Create: `plexus-frontend/src/components/MarkdownContent.tsx`

- [ ] **Step 1: Create `store/devices.ts`**

```ts
import { create } from 'zustand'
import { api } from '@/lib/api'
import type { Device } from '@/lib/types'

interface DevicesState {
  devices: Device[]
  loading: boolean
  error: string | null
  refresh: () => Promise<void>
  startPolling: () => () => void
}

export const useDevicesStore = create<DevicesState>((set, get) => ({
  devices: [],
  loading: false,
  error: null,
  async refresh() {
    set({ loading: true, error: null })
    try {
      const devices = await api<Device[]>('/api/devices')
      set({ devices, loading: false })
    } catch (e) {
      set({ error: e instanceof Error ? e.message : 'error', loading: false })
    }
  },
  startPolling() {
    let active = true
    const tick = async () => {
      if (!active) return
      await get().refresh()
      if (active) setTimeout(tick, 5_000)
    }
    tick()
    return () => {
      active = false
    }
  },
}))
```

- [ ] **Step 2: Create `components/MarkdownContent.tsx`**

```tsx
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter'
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism'

interface Props {
  text: string
}

export default function MarkdownContent({ text }: Props) {
  return (
    <div className="prose prose-invert max-w-none text-plex-text">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          code({ inline, className, children, ...props }: any) {
            const match = /language-(\w+)/.exec(className || '')
            if (!inline && match) {
              return (
                <SyntaxHighlighter
                  style={vscDarkPlus as any}
                  language={match[1]}
                  PreTag="div"
                  customStyle={{
                    background: '#0a0f18',
                    border: '1px solid #1a2332',
                    borderRadius: 6,
                    margin: '8px 0',
                    fontSize: 12,
                  }}
                  {...props}
                >
                  {String(children).replace(/\n$/, '')}
                </SyntaxHighlighter>
              )
            }
            return (
              <code
                className="rounded bg-plex-sidebar px-1 py-0.5 text-plex-accent"
                {...props}
              >
                {children}
              </code>
            )
          },
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
}
```

- [ ] **Step 3: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build. If any `prose` class is missing, that's fine — it's optional styling.

- [ ] **Step 4: Commit**

```bash
git add plexus-frontend/src/store/devices.ts plexus-frontend/src/components/MarkdownContent.tsx
git commit -m "feat(frontend): devices store and markdown component"
```

---

### Task 17: Chat Page Components (Sidebar, Message, Input, ProgressHint, DeviceStatusBar)

**Files:**
- Create: `plexus-frontend/src/components/Sidebar.tsx`
- Create: `plexus-frontend/src/components/Message.tsx`
- Create: `plexus-frontend/src/components/MessageList.tsx`
- Create: `plexus-frontend/src/components/ProgressHint.tsx`
- Create: `plexus-frontend/src/components/ChatInput.tsx`
- Create: `plexus-frontend/src/components/DeviceStatusBar.tsx`

- [ ] **Step 1: Create `Sidebar.tsx`**

```tsx
import { useState } from 'react'
import { NavLink } from 'react-router-dom'
import { ChevronLeft, ChevronRight, Plus, Settings, ShieldCheck, LogOut } from 'lucide-react'
import { useChatStore } from '@/store/chat'
import { useAuthStore } from '@/store/auth'

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false)
  const sessions = useChatStore((s) => s.sessions)
  const currentSessionId = useChatStore((s) => s.currentSessionId)
  const setCurrentSession = useChatStore((s) => s.setCurrentSession)
  const newSession = useChatStore((s) => s.newSession)
  const isAdmin = useAuthStore((s) => s.isAdmin)
  const logout = useAuthStore((s) => s.logout)

  return (
    <aside
      className={`flex flex-col border-r border-plex-border bg-plex-sidebar transition-all ${
        collapsed ? 'w-12' : 'w-[clamp(180px,16vw,220px)]'
      }`}
    >
      <div className="flex items-center justify-between px-3 py-3">
        {!collapsed && (
          <span className="text-sm font-semibold tracking-wider text-plex-accent">
            Plexus
          </span>
        )}
        <button
          onClick={() => setCollapsed((c) => !c)}
          className="text-plex-muted hover:text-plex-accent"
          title={collapsed ? 'Expand' : 'Collapse'}
        >
          {collapsed ? <ChevronRight size={14} /> : <ChevronLeft size={14} />}
        </button>
      </div>

      <button
        onClick={newSession}
        className="mx-2 mb-2 flex items-center justify-center gap-1 rounded border border-dashed border-plex-border px-2 py-1.5 text-xs text-plex-muted hover:border-plex-accent hover:text-plex-accent"
        title="New chat"
      >
        <Plus size={12} />
        {!collapsed && <span>New chat</span>}
      </button>

      {!collapsed && (
        <div className="mb-1 px-3 text-[9px] uppercase tracking-wider text-plex-muted">
          Recent
        </div>
      )}
      <div className="flex-1 overflow-y-auto px-2">
        {sessions.map((s) => {
          const isActive = s.session_id === currentSessionId
          const label = s.session_id.slice(-6)
          return (
            <button
              key={s.session_id}
              onClick={() => setCurrentSession(s.session_id)}
              className={`mb-1 w-full truncate rounded px-2 py-1.5 text-left text-xs ${
                isActive
                  ? 'border-l-2 border-plex-accent bg-plex-accent/10 text-plex-text'
                  : 'text-plex-muted hover:text-plex-text'
              }`}
              title={s.session_id}
            >
              {collapsed ? label.charAt(0).toUpperCase() : `Session ${label}`}
            </button>
          )
        })}
      </div>

      <div className="mt-auto border-t border-plex-border px-2 py-2">
        <NavLink
          to="/settings"
          className="flex items-center gap-2 rounded px-2 py-1.5 text-xs text-plex-muted hover:text-plex-accent"
        >
          <Settings size={14} />
          {!collapsed && <span>Settings</span>}
        </NavLink>
        {isAdmin && (
          <NavLink
            to="/admin"
            className="flex items-center gap-2 rounded px-2 py-1.5 text-xs text-plex-muted hover:text-plex-accent"
          >
            <ShieldCheck size={14} />
            {!collapsed && <span>Admin</span>}
          </NavLink>
        )}
        <button
          onClick={logout}
          className="flex items-center gap-2 rounded px-2 py-1.5 text-xs text-plex-muted hover:text-plex-danger"
        >
          <LogOut size={14} />
          {!collapsed && <span>Sign out</span>}
        </button>
      </div>
    </aside>
  )
}
```

- [ ] **Step 2: Create `Message.tsx`**

```tsx
import type { ChatMessage } from '@/store/chat'
import MarkdownContent from './MarkdownContent'

interface Props {
  message: ChatMessage
}

export default function Message({ message }: Props) {
  const isUser = message.role === 'user'
  return (
    <div className={`mb-4 flex ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div
        className={
          isUser
            ? 'max-w-[70%] rounded-[12px_12px_2px_12px] bg-plex-accent/10 px-4 py-2.5 text-plex-text'
            : 'max-w-[80%] rounded-[2px_12px_12px_12px] border-l-[3px] border-plex-accent bg-plex-card px-4 py-3 text-plex-text'
        }
      >
        {isUser ? <span>{message.content}</span> : <MarkdownContent text={message.content} />}
      </div>
    </div>
  )
}
```

- [ ] **Step 3: Create `MessageList.tsx`**

```tsx
import { useEffect, useRef } from 'react'
import type { ChatMessage } from '@/store/chat'
import Message from './Message'
import ProgressHint from './ProgressHint'

interface Props {
  messages: ChatMessage[]
  progressHint: string | null
}

export default function MessageList({ messages, progressHint }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages, progressHint])

  return (
    <div className="flex-1 overflow-y-auto px-6 py-4">
      {messages.map((m) => (
        <Message key={m.id} message={m} />
      ))}
      {progressHint && <ProgressHint text={progressHint} />}
      <div ref={bottomRef} />
    </div>
  )
}
```

- [ ] **Step 4: Create `ProgressHint.tsx`**

```tsx
interface Props {
  text: string
}

export default function ProgressHint({ text }: Props) {
  return (
    <div className="mb-4 flex items-center gap-2 text-xs text-plex-accent/80">
      <span className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-plex-accent/40 border-t-plex-accent" />
      <span>{text}</span>
    </div>
  )
}
```

- [ ] **Step 5: Create `ChatInput.tsx`**

```tsx
import { FormEvent, KeyboardEvent, useState } from 'react'
import { Send } from 'lucide-react'

interface Props {
  onSend: (text: string) => void
  disabled?: boolean
}

export default function ChatInput({ onSend, disabled }: Props) {
  const [text, setText] = useState('')

  function submit(e?: FormEvent) {
    e?.preventDefault()
    const trimmed = text.trim()
    if (!trimmed || disabled) return
    onSend(trimmed)
    setText('')
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      submit()
    }
  }

  return (
    <form
      onSubmit={submit}
      className="mx-auto w-full max-w-[clamp(420px,60vw,720px)] rounded-[10px] border border-plex-border bg-plex-card px-4 py-3"
    >
      <div className="flex items-center gap-3">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          rows={1}
          placeholder="Message Plexus…"
          className="flex-1 resize-none bg-transparent text-sm text-plex-text placeholder:text-plex-muted focus:outline-none"
          disabled={disabled}
        />
        <button
          type="submit"
          disabled={disabled || !text.trim()}
          className="text-plex-accent hover:brightness-110 disabled:opacity-40"
          title="Send"
        >
          <Send size={16} />
        </button>
      </div>
    </form>
  )
}
```

- [ ] **Step 6: Create `DeviceStatusBar.tsx`**

```tsx
import { useEffect } from 'react'
import { useDevicesStore } from '@/store/devices'
import { useChatStore } from '@/store/chat'

export default function DeviceStatusBar() {
  const devices = useDevicesStore((s) => s.devices)
  const startPolling = useDevicesStore((s) => s.startPolling)
  const wsStatus = useChatStore((s) => s.wsStatus)

  useEffect(() => {
    return startPolling()
  }, [startPolling])

  return (
    <div className="flex items-center gap-3 text-xs text-plex-muted">
      <Dot active={wsStatus === 'open'} label="gateway" />
      {devices.map((d) => (
        <Dot key={d.device_name} active={d.status === 'online'} label={d.device_name} />
      ))}
    </div>
  )
}

function Dot({ active, label }: { active: boolean; label: string }) {
  return (
    <div className="flex items-center gap-1">
      <span
        className={`inline-block h-1.5 w-1.5 rounded-full ${
          active ? 'bg-plex-accent shadow-[0_0_6px_rgba(57,255,20,0.6)]' : 'bg-plex-danger'
        }`}
      />
      <span>{label}</span>
    </div>
  )
}
```

- [ ] **Step 7: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 8: Commit**

```bash
git add plexus-frontend/src/components/
git commit -m "feat(frontend): chat page components (sidebar, messages, input, progress)"
```

---

### Task 18: Wire Up the Chat Page

**Files:**
- Replace: `plexus-frontend/src/pages/Chat.tsx`

- [ ] **Step 1: Implement Chat page**

```tsx
import { useEffect } from 'react'
import { useAuthStore } from '@/store/auth'
import { useChatStore } from '@/store/chat'
import { wsManager } from '@/lib/ws'
import Sidebar from '@/components/Sidebar'
import MessageList from '@/components/MessageList'
import ChatInput from '@/components/ChatInput'
import DeviceStatusBar from '@/components/DeviceStatusBar'

export default function Chat() {
  const token = useAuthStore((s) => s.token)
  const init = useChatStore((s) => s.init)
  const loadSessions = useChatStore((s) => s.loadSessions)
  const loadMessages = useChatStore((s) => s.loadMessages)
  const sendMessage = useChatStore((s) => s.sendMessage)
  const currentSessionId = useChatStore((s) => s.currentSessionId)
  const messagesBySession = useChatStore((s) => s.messagesBySession)
  const progressBySession = useChatStore((s) => s.progressBySession)

  useEffect(() => {
    init()
    if (token) wsManager.connect(token)
    loadSessions().catch(console.error)
    return () => {
      wsManager.disconnect()
    }
  }, [token, init, loadSessions])

  useEffect(() => {
    if (currentSessionId && !messagesBySession[currentSessionId]) {
      loadMessages(currentSessionId).catch(console.error)
    }
  }, [currentSessionId, messagesBySession, loadMessages])

  const messages = currentSessionId ? messagesBySession[currentSessionId] ?? [] : []
  const progress = currentSessionId ? progressBySession[currentSessionId] ?? null : null
  const empty = messages.length === 0

  return (
    <div className="flex h-screen w-screen bg-plex-bg text-plex-text">
      <Sidebar />
      <main className="flex flex-1 flex-col">
        <header className="flex items-center justify-between border-b border-plex-border px-6 py-3">
          <span className="text-sm font-medium">
            {currentSessionId ? `Session ${currentSessionId.slice(-6)}` : 'New chat'}
          </span>
          <DeviceStatusBar />
        </header>

        {empty ? (
          <div className="flex flex-1 flex-col items-center justify-center">
            <h1 className="mb-8 text-3xl font-light text-plex-text">
              What are we building today?
            </h1>
            <div className="w-full px-6">
              <ChatInput onSend={sendMessage} />
            </div>
          </div>
        ) : (
          <>
            <MessageList messages={messages} progressHint={progress} />
            <div className="border-t border-plex-border px-6 py-3">
              <ChatInput onSend={sendMessage} />
            </div>
          </>
        )}
      </main>
    </div>
  )
}
```

- [ ] **Step 2: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add plexus-frontend/src/pages/Chat.tsx
git commit -m "feat(frontend): wire up chat page with empty and active states"
```

---

### Task 19: Settings Page — Profile Tab

**Files:**
- Create: `plexus-frontend/src/components/Tabs.tsx`
- Replace: `plexus-frontend/src/pages/Settings.tsx`

- [ ] **Step 1: Create reusable `Tabs.tsx`**

```tsx
import { ReactNode, useState } from 'react'

export interface TabDef {
  id: string
  label: string
  content: ReactNode
}

export default function Tabs({ tabs, initial }: { tabs: TabDef[]; initial?: string }) {
  const [active, setActive] = useState(initial ?? tabs[0]?.id)
  const current = tabs.find((t) => t.id === active)
  return (
    <div className="flex h-full flex-col">
      <div className="flex gap-1 border-b border-plex-border px-6">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setActive(t.id)}
            className={`px-4 py-3 text-sm ${
              active === t.id
                ? 'border-b-2 border-plex-accent text-plex-text'
                : 'text-plex-muted hover:text-plex-text'
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="flex-1 overflow-y-auto px-6 py-6">{current?.content}</div>
    </div>
  )
}
```

- [ ] **Step 2: Create `Settings.tsx` with Profile tab wired up**

```tsx
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { ChevronLeft } from 'lucide-react'
import Tabs from '@/components/Tabs'
import { api } from '@/lib/api'
import type { UserProfile } from '@/lib/types'

function ProfileTab() {
  const [profile, setProfile] = useState<UserProfile | null>(null)
  const [soul, setSoul] = useState('')
  const [memory, setMemory] = useState('')
  const [savingSoul, setSavingSoul] = useState(false)
  const [savingMem, setSavingMem] = useState(false)
  const [toast, setToast] = useState<string | null>(null)

  useEffect(() => {
    ;(async () => {
      const p = await api<UserProfile>('/api/user/profile')
      setProfile(p)
      const s = await api<{ soul: string | null }>('/api/user/soul')
      setSoul(s.soul ?? '')
      const m = await api<{ memory: string | null }>('/api/user/memory')
      setMemory(m.memory ?? '')
    })().catch(console.error)
  }, [])

  async function saveSoul() {
    setSavingSoul(true)
    try {
      await api('/api/user/soul', { method: 'PATCH', json: { soul } })
      setToast('Soul saved')
    } finally {
      setSavingSoul(false)
    }
  }

  async function saveMemory() {
    setSavingMem(true)
    try {
      await api('/api/user/memory', { method: 'PATCH', json: { memory } })
      setToast('Memory saved')
    } finally {
      setSavingMem(false)
    }
  }

  return (
    <div className="space-y-6">
      {profile && (
        <div className="rounded border border-plex-border bg-plex-card p-4">
          <div className="text-xs uppercase tracking-wider text-plex-muted">Account</div>
          <div className="mt-2 text-sm">{profile.email}</div>
          <div className="text-xs text-plex-muted">
            {profile.user_id} {profile.is_admin && '· admin'}
          </div>
        </div>
      )}
      <div>
        <label className="mb-2 block text-xs uppercase tracking-wider text-plex-muted">
          Soul (custom system prompt)
        </label>
        <textarea
          value={soul}
          onChange={(e) => setSoul(e.target.value)}
          rows={8}
          className="w-full rounded border border-plex-border bg-plex-sidebar p-3 font-mono text-sm text-plex-text focus:border-plex-accent focus:outline-none"
        />
        <button
          onClick={saveSoul}
          disabled={savingSoul}
          className="mt-2 rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black hover:brightness-110 disabled:opacity-50"
        >
          {savingSoul ? 'Saving…' : 'Save soul'}
        </button>
      </div>
      <div>
        <label className="mb-2 flex items-center justify-between text-xs uppercase tracking-wider text-plex-muted">
          <span>Memory (4K char cap)</span>
          <span>{memory.length} / 4096</span>
        </label>
        <textarea
          value={memory}
          onChange={(e) => setMemory(e.target.value)}
          rows={8}
          maxLength={4096}
          className="w-full rounded border border-plex-border bg-plex-sidebar p-3 font-mono text-sm text-plex-text focus:border-plex-accent focus:outline-none"
        />
        <button
          onClick={saveMemory}
          disabled={savingMem}
          className="mt-2 rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black hover:brightness-110 disabled:opacity-50"
        >
          {savingMem ? 'Saving…' : 'Save memory'}
        </button>
      </div>
      {toast && <div className="text-xs text-plex-accent">{toast}</div>}
    </div>
  )
}

function Placeholder({ title }: { title: string }) {
  return <div className="text-sm text-plex-muted">{title} — coming next</div>
}

export default function Settings() {
  return (
    <div className="flex h-screen flex-col bg-plex-bg text-plex-text">
      <header className="flex items-center gap-2 border-b border-plex-border px-6 py-3">
        <Link to="/chat" className="text-plex-muted hover:text-plex-accent">
          <ChevronLeft size={16} />
        </Link>
        <span className="text-sm font-medium">Settings</span>
      </header>
      <div className="flex-1">
        <Tabs
          tabs={[
            { id: 'profile', label: 'Profile', content: <ProfileTab /> },
            { id: 'devices', label: 'Devices', content: <Placeholder title="Devices" /> },
            { id: 'channels', label: 'Channels', content: <Placeholder title="Channels" /> },
            { id: 'skills', label: 'Skills', content: <Placeholder title="Skills" /> },
            { id: 'cron', label: 'Cron', content: <Placeholder title="Cron" /> },
          ]}
        />
      </div>
    </div>
  )
}
```

- [ ] **Step 3: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 4: Commit**

```bash
git add plexus-frontend/src/components/Tabs.tsx plexus-frontend/src/pages/Settings.tsx
git commit -m "feat(frontend): settings page with profile tab"
```

---

### Task 20: Settings — Devices Tab

**Files:**
- Modify: `plexus-frontend/src/pages/Settings.tsx`

- [ ] **Step 1: Replace the Placeholder for devices with a real `DevicesTab`**

Add inside `Settings.tsx` (above the `Placeholder` function):

```tsx
import type { Device, DeviceToken, McpServerEntry } from '@/lib/types'

function DevicesTab() {
  const [devices, setDevices] = useState<Device[]>([])
  const [tokens, setTokens] = useState<DeviceToken[]>([])
  const [newName, setNewName] = useState('')
  const [newTokenValue, setNewTokenValue] = useState<string | null>(null)
  const [expanded, setExpanded] = useState<string | null>(null)
  const [policyByDevice, setPolicyByDevice] = useState<Record<string, string>>({})
  const [mcpByDevice, setMcpByDevice] = useState<Record<string, string>>({})

  async function refresh() {
    const [ds, ts] = await Promise.all([
      api<Device[]>('/api/devices'),
      api<DeviceToken[]>('/api/device-tokens'),
    ])
    setDevices(ds)
    setTokens(ts)
  }

  useEffect(() => {
    refresh().catch(console.error)
    const id = window.setInterval(refresh, 5000)
    return () => window.clearInterval(id)
  }, [])

  async function createToken() {
    if (!newName.trim()) return
    const res = await api<DeviceToken>('/api/device-tokens', {
      method: 'POST',
      json: { device_name: newName.trim() },
    })
    setNewTokenValue(res.token)
    setNewName('')
    refresh()
  }

  async function deleteToken(token: string) {
    if (!confirm('Delete this device token?')) return
    await api(`/api/device-tokens/${encodeURIComponent(token)}`, { method: 'DELETE' })
    refresh()
  }

  async function expandDevice(name: string) {
    if (expanded === name) {
      setExpanded(null)
      return
    }
    setExpanded(name)
    if (!policyByDevice[name]) {
      const pol = await api<{ fs_policy: { mode: string } }>(
        `/api/devices/${encodeURIComponent(name)}/policy`,
      )
      setPolicyByDevice((s) => ({ ...s, [name]: pol.fs_policy.mode }))
    }
    if (!mcpByDevice[name]) {
      const mcp = await api<{ mcp_servers: McpServerEntry[] }>(
        `/api/devices/${encodeURIComponent(name)}/mcp`,
      )
      setMcpByDevice((s) => ({
        ...s,
        [name]: JSON.stringify(mcp.mcp_servers, null, 2),
      }))
    }
  }

  async function savePolicy(name: string) {
    await api(`/api/devices/${encodeURIComponent(name)}/policy`, {
      method: 'PATCH',
      json: { fs_policy: { mode: policyByDevice[name] } },
    })
  }

  async function saveMcp(name: string) {
    let parsed: McpServerEntry[]
    try {
      parsed = JSON.parse(mcpByDevice[name])
    } catch {
      alert('Invalid JSON')
      return
    }
    await api(`/api/devices/${encodeURIComponent(name)}/mcp`, {
      method: 'PUT',
      json: { mcp_servers: parsed },
    })
  }

  return (
    <div className="space-y-6">
      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">
          Registered Devices
        </h3>
        <div className="space-y-2">
          {devices.length === 0 && (
            <div className="text-xs text-plex-muted">No devices registered yet.</div>
          )}
          {devices.map((d) => (
            <div
              key={d.device_name}
              className="rounded border border-plex-border bg-plex-card"
            >
              <button
                onClick={() => expandDevice(d.device_name)}
                className="flex w-full items-center justify-between px-4 py-3 text-left"
              >
                <div className="flex items-center gap-3">
                  <span
                    className={`inline-block h-2 w-2 rounded-full ${
                      d.status === 'online'
                        ? 'bg-plex-accent shadow-[0_0_6px_rgba(57,255,20,0.6)]'
                        : 'bg-plex-danger'
                    }`}
                  />
                  <span className="text-sm font-medium">{d.device_name}</span>
                  <span className="text-xs text-plex-muted">
                    {d.tools_count} tools · {d.fs_policy.mode}
                  </span>
                </div>
                <span className="text-xs text-plex-muted">
                  {d.status === 'online' ? 'online' : `${d.last_seen_secs_ago}s ago`}
                </span>
              </button>
              {expanded === d.device_name && (
                <div className="space-y-4 border-t border-plex-border px-4 py-3">
                  <div>
                    <label className="mb-1 block text-xs uppercase tracking-wider text-plex-muted">
                      Sandbox Policy
                    </label>
                    <select
                      value={policyByDevice[d.device_name] ?? 'sandbox'}
                      onChange={(e) =>
                        setPolicyByDevice((s) => ({
                          ...s,
                          [d.device_name]: e.target.value,
                        }))
                      }
                      className="rounded border border-plex-border bg-plex-sidebar px-2 py-1 text-sm"
                    >
                      <option value="sandbox">sandbox</option>
                      <option value="unrestricted">unrestricted</option>
                    </select>
                    <button
                      onClick={() => savePolicy(d.device_name)}
                      className="ml-2 rounded bg-plex-accent px-3 py-1 text-xs font-semibold text-black"
                    >
                      Save
                    </button>
                  </div>
                  <div>
                    <label className="mb-1 block text-xs uppercase tracking-wider text-plex-muted">
                      MCP Servers (JSON)
                    </label>
                    <textarea
                      value={mcpByDevice[d.device_name] ?? '[]'}
                      onChange={(e) =>
                        setMcpByDevice((s) => ({
                          ...s,
                          [d.device_name]: e.target.value,
                        }))
                      }
                      rows={6}
                      className="w-full rounded border border-plex-border bg-plex-sidebar p-2 font-mono text-xs"
                    />
                    <button
                      onClick={() => saveMcp(d.device_name)}
                      className="mt-1 rounded bg-plex-accent px-3 py-1 text-xs font-semibold text-black"
                    >
                      Save MCP
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">
          Device Tokens
        </h3>
        <div className="mb-3 flex gap-2">
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder="device-name"
            className="flex-1 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
          />
          <button
            onClick={createToken}
            className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
          >
            Create token
          </button>
        </div>
        {newTokenValue && (
          <div className="mb-3 rounded border border-plex-accent/40 bg-plex-accent/5 p-3 text-xs">
            <div className="mb-1 font-semibold text-plex-accent">New token (copy now):</div>
            <code className="break-all text-plex-text">{newTokenValue}</code>
            <button
              onClick={() => {
                navigator.clipboard.writeText(newTokenValue)
              }}
              className="ml-2 text-plex-accent underline"
            >
              copy
            </button>
          </div>
        )}
        <ul className="space-y-1">
          {tokens.map((t) => (
            <li
              key={t.token}
              className="flex items-center justify-between rounded border border-plex-border bg-plex-card px-3 py-2 text-xs"
            >
              <span>{t.device_name}</span>
              <button
                onClick={() => deleteToken(t.token)}
                className="text-plex-danger hover:underline"
              >
                delete
              </button>
            </li>
          ))}
        </ul>
      </section>
    </div>
  )
}
```

- [ ] **Step 2: Replace the `devices` tab entry with `<DevicesTab />`**

In the `Tabs` array:

```tsx
{ id: 'devices', label: 'Devices', content: <DevicesTab /> },
```

- [ ] **Step 3: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 4: Commit**

```bash
git add plexus-frontend/src/pages/Settings.tsx
git commit -m "feat(frontend): settings devices tab (status, tokens, policy, mcp)"
```

---

### Task 21: Settings — Channels, Skills, Cron Tabs

**Files:**
- Modify: `plexus-frontend/src/pages/Settings.tsx`

- [ ] **Step 1: Add `ChannelsTab`, `SkillsTab`, `CronTab`**

Add the following components inside `Settings.tsx` (below `DevicesTab`):

```tsx
import type { DiscordConfig, TelegramConfig, Skill, CronJob } from '@/lib/types'

function ChannelsTab() {
  const [discord, setDiscord] = useState<DiscordConfig | null>(null)
  const [telegram, setTelegram] = useState<TelegramConfig | null>(null)
  const [discordForm, setDiscordForm] = useState({
    bot_token: '',
    allowed_users: '',
    owner_discord_id: '',
  })
  const [telegramForm, setTelegramForm] = useState({
    bot_token: '',
    partner_telegram_id: '',
    allowed_users: '',
    group_policy: 'mention' as 'mention' | 'all',
  })

  useEffect(() => {
    ;(async () => {
      try {
        setDiscord(await api<DiscordConfig>('/api/discord-config'))
      } catch {
        setDiscord(null)
      }
      try {
        setTelegram(await api<TelegramConfig>('/api/telegram-config'))
      } catch {
        setTelegram(null)
      }
    })()
  }, [])

  async function saveDiscord() {
    const body = {
      bot_token: discordForm.bot_token,
      allowed_users: discordForm.allowed_users
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
      owner_discord_id: discordForm.owner_discord_id || undefined,
    }
    const res = await api<DiscordConfig>('/api/discord-config', {
      method: 'POST',
      json: body,
    })
    setDiscord(res)
  }

  async function deleteDiscord() {
    if (!confirm('Delete Discord config?')) return
    await api('/api/discord-config', { method: 'DELETE' })
    setDiscord(null)
  }

  async function saveTelegram() {
    const body = {
      bot_token: telegramForm.bot_token,
      partner_telegram_id: telegramForm.partner_telegram_id,
      allowed_users: telegramForm.allowed_users
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
      group_policy: telegramForm.group_policy,
    }
    const res = await api<TelegramConfig>('/api/telegram-config', {
      method: 'POST',
      json: body,
    })
    setTelegram(res)
  }

  async function deleteTelegram() {
    if (!confirm('Delete Telegram config?')) return
    await api('/api/telegram-config', { method: 'DELETE' })
    setTelegram(null)
  }

  return (
    <div className="grid grid-cols-1 gap-8 lg:grid-cols-2">
      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">Discord</h3>
        {discord ? (
          <div className="rounded border border-plex-border bg-plex-card p-4 text-sm">
            <div>Status: {discord.enabled ? 'enabled' : 'disabled'}</div>
            <div>Allowed users: {discord.allowed_users.join(', ') || '—'}</div>
            <button
              onClick={deleteDiscord}
              className="mt-3 rounded border border-plex-danger px-3 py-1 text-xs text-plex-danger"
            >
              Delete config
            </button>
          </div>
        ) : (
          <div className="space-y-2 rounded border border-plex-border bg-plex-card p-4">
            <input
              type="password"
              placeholder="Bot token"
              value={discordForm.bot_token}
              onChange={(e) =>
                setDiscordForm({ ...discordForm, bot_token: e.target.value })
              }
              className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
            <input
              placeholder="Owner Discord ID"
              value={discordForm.owner_discord_id}
              onChange={(e) =>
                setDiscordForm({ ...discordForm, owner_discord_id: e.target.value })
              }
              className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
            <input
              placeholder="Allowed user IDs (comma-separated)"
              value={discordForm.allowed_users}
              onChange={(e) =>
                setDiscordForm({ ...discordForm, allowed_users: e.target.value })
              }
              className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
            <button
              onClick={saveDiscord}
              className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
            >
              Save
            </button>
          </div>
        )}
      </section>

      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">Telegram</h3>
        {telegram ? (
          <div className="rounded border border-plex-border bg-plex-card p-4 text-sm">
            <div>Status: {telegram.enabled ? 'enabled' : 'disabled'}</div>
            <div>Partner: {telegram.partner_telegram_id}</div>
            <div>Group policy: {telegram.group_policy}</div>
            <button
              onClick={deleteTelegram}
              className="mt-3 rounded border border-plex-danger px-3 py-1 text-xs text-plex-danger"
            >
              Delete config
            </button>
          </div>
        ) : (
          <div className="space-y-2 rounded border border-plex-border bg-plex-card p-4">
            <input
              type="password"
              placeholder="Bot token"
              value={telegramForm.bot_token}
              onChange={(e) =>
                setTelegramForm({ ...telegramForm, bot_token: e.target.value })
              }
              className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
            <input
              placeholder="Partner Telegram ID"
              value={telegramForm.partner_telegram_id}
              onChange={(e) =>
                setTelegramForm({ ...telegramForm, partner_telegram_id: e.target.value })
              }
              className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
            <input
              placeholder="Allowed user IDs (comma-separated)"
              value={telegramForm.allowed_users}
              onChange={(e) =>
                setTelegramForm({ ...telegramForm, allowed_users: e.target.value })
              }
              className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
            <select
              value={telegramForm.group_policy}
              onChange={(e) =>
                setTelegramForm({
                  ...telegramForm,
                  group_policy: e.target.value as 'mention' | 'all',
                })
              }
              className="rounded border border-plex-border bg-plex-sidebar px-2 py-1 text-sm"
            >
              <option value="mention">mention</option>
              <option value="all">all</option>
            </select>
            <button
              onClick={saveTelegram}
              className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
            >
              Save
            </button>
          </div>
        )}
      </section>
    </div>
  )
}

function SkillsTab() {
  const [skills, setSkills] = useState<Skill[]>([])
  const [repo, setRepo] = useState('')
  const [branch, setBranch] = useState('main')
  const [content, setContent] = useState('')

  async function refresh() {
    const res = await api<{ skills: Skill[] }>('/api/skills')
    setSkills(res.skills)
  }

  useEffect(() => {
    refresh().catch(console.error)
  }, [])

  async function installFromRepo() {
    if (!repo.trim()) return
    await api('/api/skills/install', {
      method: 'POST',
      json: { repo: repo.trim(), branch: branch.trim() || 'main' },
    })
    setRepo('')
    refresh()
  }

  async function uploadContent() {
    if (!content.trim()) return
    await api('/api/skills', {
      method: 'POST',
      json: { name: '', content },
    })
    setContent('')
    refresh()
  }

  async function deleteSkill(name: string) {
    if (!confirm(`Delete skill "${name}"?`)) return
    await api(`/api/skills/${encodeURIComponent(name)}`, { method: 'DELETE' })
    refresh()
  }

  return (
    <div className="space-y-6">
      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">
          Installed Skills
        </h3>
        <ul className="space-y-1">
          {skills.map((s) => (
            <li
              key={s.skill_id}
              className="flex items-start justify-between rounded border border-plex-border bg-plex-card p-3"
            >
              <div>
                <div className="text-sm font-medium">{s.name}</div>
                <div className="text-xs text-plex-muted">{s.description}</div>
                {s.always_on && (
                  <div className="mt-1 text-[10px] uppercase tracking-wider text-plex-accent">
                    always on
                  </div>
                )}
              </div>
              <button
                onClick={() => deleteSkill(s.name)}
                className="text-xs text-plex-danger hover:underline"
              >
                delete
              </button>
            </li>
          ))}
          {skills.length === 0 && (
            <li className="text-xs text-plex-muted">No skills installed.</li>
          )}
        </ul>
      </section>

      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">
          Install from GitHub
        </h3>
        <div className="flex gap-2">
          <input
            value={repo}
            onChange={(e) => setRepo(e.target.value)}
            placeholder="owner/repo"
            className="flex-1 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
          />
          <input
            value={branch}
            onChange={(e) => setBranch(e.target.value)}
            placeholder="branch"
            className="w-32 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
          />
          <button
            onClick={installFromRepo}
            className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
          >
            Install
          </button>
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">
          Paste SKILL.md
        </h3>
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={8}
          placeholder="---&#10;name: my-skill&#10;description: ...&#10;---&#10;&#10;Instructions"
          className="w-full rounded border border-plex-border bg-plex-sidebar p-3 font-mono text-xs"
        />
        <button
          onClick={uploadContent}
          className="mt-2 rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
        >
          Upload
        </button>
      </section>
    </div>
  )
}

function CronTab() {
  const [jobs, setJobs] = useState<CronJob[]>([])
  const [form, setForm] = useState({
    name: '',
    message: '',
    schedule_kind: 'cron' as 'cron' | 'every' | 'at',
    cron_expr: '',
    every_seconds: '',
    at: '',
    channel: 'webui',
    timezone: 'UTC',
  })

  async function refresh() {
    const res = await api<{ cron_jobs: CronJob[] }>('/api/cron-jobs')
    setJobs(res.cron_jobs)
  }

  useEffect(() => {
    refresh().catch(console.error)
  }, [])

  async function create() {
    const body: Record<string, unknown> = {
      message: form.message,
      channel: form.channel,
      name: form.name || undefined,
      timezone: form.timezone,
      delete_after_run: false,
    }
    if (form.schedule_kind === 'cron') body.cron_expr = form.cron_expr
    if (form.schedule_kind === 'every') body.every_seconds = Number(form.every_seconds)
    if (form.schedule_kind === 'at') body.at = form.at
    await api('/api/cron-jobs', { method: 'POST', json: body })
    setForm({ ...form, message: '', name: '', cron_expr: '', every_seconds: '', at: '' })
    refresh()
  }

  async function toggle(job: CronJob) {
    await api(`/api/cron-jobs/${job.job_id}`, {
      method: 'PATCH',
      json: { enabled: !job.enabled },
    })
    refresh()
  }

  async function del(job: CronJob) {
    if (!confirm(`Delete job ${job.job_id}?`)) return
    await api(`/api/cron-jobs/${job.job_id}`, { method: 'DELETE' })
    refresh()
  }

  return (
    <div className="space-y-6">
      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">Jobs</h3>
        <div className="space-y-2">
          {jobs.map((j) => (
            <div
              key={j.job_id}
              className="rounded border border-plex-border bg-plex-card p-3 text-sm"
            >
              <div className="flex items-center justify-between">
                <div>
                  <div className="font-medium">{j.name ?? j.job_id}</div>
                  <div className="text-xs text-plex-muted">
                    {j.cron_expr ?? (j.every_seconds ? `every ${j.every_seconds}s` : 'one-shot')}
                    {' · '}
                    {j.channel}
                  </div>
                  <div className="mt-1 text-xs text-plex-muted">{j.message}</div>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => toggle(j)}
                    className={`rounded border px-2 py-1 text-xs ${
                      j.enabled
                        ? 'border-plex-accent text-plex-accent'
                        : 'border-plex-border text-plex-muted'
                    }`}
                  >
                    {j.enabled ? 'enabled' : 'disabled'}
                  </button>
                  <button
                    onClick={() => del(j)}
                    className="text-xs text-plex-danger hover:underline"
                  >
                    delete
                  </button>
                </div>
              </div>
            </div>
          ))}
          {jobs.length === 0 && <div className="text-xs text-plex-muted">No jobs.</div>}
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wider text-plex-muted">Create job</h3>
        <div className="space-y-2">
          <input
            value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            placeholder="Name (optional)"
            className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
          />
          <textarea
            value={form.message}
            onChange={(e) => setForm({ ...form, message: e.target.value })}
            placeholder="Agent instruction"
            rows={3}
            className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
          />
          <div className="flex gap-2">
            <select
              value={form.schedule_kind}
              onChange={(e) =>
                setForm({
                  ...form,
                  schedule_kind: e.target.value as 'cron' | 'every' | 'at',
                })
              }
              className="rounded border border-plex-border bg-plex-sidebar px-2 py-1.5 text-sm"
            >
              <option value="cron">cron expr</option>
              <option value="every">every N sec</option>
              <option value="at">one-shot</option>
            </select>
            {form.schedule_kind === 'cron' && (
              <input
                value={form.cron_expr}
                onChange={(e) => setForm({ ...form, cron_expr: e.target.value })}
                placeholder="0 9 * * *"
                className="flex-1 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
              />
            )}
            {form.schedule_kind === 'every' && (
              <input
                type="number"
                value={form.every_seconds}
                onChange={(e) => setForm({ ...form, every_seconds: e.target.value })}
                placeholder="1800"
                className="flex-1 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
              />
            )}
            {form.schedule_kind === 'at' && (
              <input
                type="datetime-local"
                value={form.at}
                onChange={(e) => setForm({ ...form, at: e.target.value })}
                className="flex-1 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
              />
            )}
          </div>
          <div className="flex gap-2">
            <input
              value={form.channel}
              onChange={(e) => setForm({ ...form, channel: e.target.value })}
              placeholder="channel"
              className="flex-1 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
            <input
              value={form.timezone}
              onChange={(e) => setForm({ ...form, timezone: e.target.value })}
              placeholder="UTC"
              className="w-32 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
            />
          </div>
          <button
            onClick={create}
            className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
          >
            Create
          </button>
        </div>
      </section>
    </div>
  )
}
```

- [ ] **Step 2: Replace the remaining placeholders in the `Tabs` array**

```tsx
{ id: 'channels', label: 'Channels', content: <ChannelsTab /> },
{ id: 'skills', label: 'Skills', content: <SkillsTab /> },
{ id: 'cron', label: 'Cron', content: <CronTab /> },
```

- [ ] **Step 3: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 4: Commit**

```bash
git add plexus-frontend/src/pages/Settings.tsx
git commit -m "feat(frontend): settings channels, skills, and cron tabs"
```

---

### Task 22: Admin Page

**Files:**
- Replace: `plexus-frontend/src/pages/Admin.tsx`

- [ ] **Step 1: Implement Admin page**

```tsx
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { ChevronLeft } from 'lucide-react'
import Tabs from '@/components/Tabs'
import { api } from '@/lib/api'
import type { LlmConfig, McpServerEntry } from '@/lib/types'

function LlmTab() {
  const [config, setConfig] = useState<LlmConfig | null>(null)
  const [status, setStatus] = useState<string | null>(null)
  const [form, setForm] = useState({
    api_base: '',
    model: '',
    api_key: '',
    context_window: '',
  })

  useEffect(() => {
    ;(async () => {
      const res = await api<any>('/api/llm-config')
      if (res.status === 'not_configured') {
        setStatus('not configured')
      } else {
        setConfig(res as LlmConfig)
        setForm({
          api_base: res.api_base,
          model: res.model,
          api_key: '',
          context_window: String(res.context_window),
        })
      }
    })().catch(console.error)
  }, [])

  async function save() {
    const body: Record<string, unknown> = { api_base: form.api_base }
    if (form.model) body.model = form.model
    if (form.api_key) body.api_key = form.api_key
    if (form.context_window) body.context_window = Number(form.context_window)
    await api('/api/llm-config', { method: 'PUT', json: body })
    setStatus('saved')
  }

  return (
    <div className="max-w-xl space-y-3">
      {status && <div className="text-xs text-plex-muted">{status}</div>}
      {config && (
        <div className="text-xs text-plex-muted">
          Current API key: <code className="text-plex-accent">{config.api_key}</code>
        </div>
      )}
      <input
        value={form.api_base}
        onChange={(e) => setForm({ ...form, api_base: e.target.value })}
        placeholder="api_base (required)"
        className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
      />
      <input
        value={form.model}
        onChange={(e) => setForm({ ...form, model: e.target.value })}
        placeholder="model (e.g. gpt-4o)"
        className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
      />
      <input
        type="password"
        value={form.api_key}
        onChange={(e) => setForm({ ...form, api_key: e.target.value })}
        placeholder="api_key (leave blank to keep existing)"
        className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
      />
      <input
        type="number"
        value={form.context_window}
        onChange={(e) => setForm({ ...form, context_window: e.target.value })}
        placeholder="context_window"
        className="w-full rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
      />
      <button
        onClick={save}
        className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
      >
        Save LLM config
      </button>
    </div>
  )
}

function DefaultSoulTab() {
  const [soul, setSoul] = useState('')
  const [toast, setToast] = useState<string | null>(null)

  useEffect(() => {
    ;(async () => {
      const res = await api<{ default_soul: string | null }>('/api/admin/default-soul')
      setSoul(res.default_soul ?? '')
    })().catch(console.error)
  }, [])

  async function save() {
    await api('/api/admin/default-soul', { method: 'PUT', json: { soul } })
    setToast('saved')
  }

  return (
    <div className="space-y-3">
      <textarea
        value={soul}
        onChange={(e) => setSoul(e.target.value)}
        rows={16}
        className="w-full rounded border border-plex-border bg-plex-sidebar p-3 font-mono text-sm"
      />
      <button
        onClick={save}
        className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
      >
        Save default soul
      </button>
      {toast && <div className="text-xs text-plex-accent">{toast}</div>}
    </div>
  )
}

function RateLimitTab() {
  const [rate, setRate] = useState(0)
  const [toast, setToast] = useState<string | null>(null)

  useEffect(() => {
    ;(async () => {
      const res = await api<{ rate_limit_per_min: number }>('/api/admin/rate-limit')
      setRate(res.rate_limit_per_min)
    })().catch(console.error)
  }, [])

  async function save() {
    await api('/api/admin/rate-limit', {
      method: 'PUT',
      json: { rate_limit_per_min: rate },
    })
    setToast('saved')
  }

  return (
    <div className="space-y-3">
      <label className="block text-xs uppercase tracking-wider text-plex-muted">
        Rate limit per minute (0 = unlimited)
      </label>
      <input
        type="number"
        value={rate}
        onChange={(e) => setRate(Number(e.target.value))}
        className="w-32 rounded border border-plex-border bg-plex-sidebar px-3 py-1.5 text-sm"
      />
      <div>
        <button
          onClick={save}
          className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
        >
          Save
        </button>
      </div>
      {toast && <div className="text-xs text-plex-accent">{toast}</div>}
    </div>
  )
}

function ServerMcpTab() {
  const [text, setText] = useState('[]')
  const [toast, setToast] = useState<string | null>(null)

  useEffect(() => {
    ;(async () => {
      const res = await api<{ mcp_servers: McpServerEntry[] }>('/api/server-mcp')
      setText(JSON.stringify(res.mcp_servers, null, 2))
    })().catch(console.error)
  }, [])

  async function save() {
    let parsed: McpServerEntry[]
    try {
      parsed = JSON.parse(text)
    } catch {
      alert('Invalid JSON')
      return
    }
    await api('/api/server-mcp', { method: 'PUT', json: { mcp_servers: parsed } })
    setToast('saved')
  }

  return (
    <div className="space-y-3">
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={16}
        className="w-full rounded border border-plex-border bg-plex-sidebar p-3 font-mono text-xs"
      />
      <button
        onClick={save}
        className="rounded bg-plex-accent px-4 py-1.5 text-sm font-semibold text-black"
      >
        Save server MCP
      </button>
      {toast && <div className="text-xs text-plex-accent">{toast}</div>}
    </div>
  )
}

export default function Admin() {
  return (
    <div className="flex h-screen flex-col bg-plex-bg text-plex-text">
      <header className="flex items-center gap-2 border-b border-plex-border px-6 py-3">
        <Link to="/chat" className="text-plex-muted hover:text-plex-accent">
          <ChevronLeft size={16} />
        </Link>
        <span className="text-sm font-medium">Admin</span>
      </header>
      <div className="flex-1">
        <Tabs
          tabs={[
            { id: 'llm', label: 'LLM', content: <LlmTab /> },
            { id: 'soul', label: 'Default Soul', content: <DefaultSoulTab /> },
            { id: 'rate', label: 'Rate Limit', content: <RateLimitTab /> },
            { id: 'mcp', label: 'Server MCP', content: <ServerMcpTab /> },
          ]}
        />
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add plexus-frontend/src/pages/Admin.tsx
git commit -m "feat(frontend): admin page (LLM, default soul, rate limit, server MCP)"
```

---

### Task 23: Component Smoke Tests

**Files:**
- Create: `plexus-frontend/src/components/__tests__/Message.test.tsx`
- Create: `plexus-frontend/src/components/__tests__/ChatInput.test.tsx`
- Create: `plexus-frontend/src/components/__tests__/ProgressHint.test.tsx`

- [ ] **Step 1: Test `Message`**

```tsx
import { render, screen } from '@testing-library/react'
import Message from '../Message'
import type { ChatMessage } from '@/store/chat'

test('renders user bubble on the right', () => {
  const m: ChatMessage = {
    id: '1',
    role: 'user',
    content: 'hello world',
    created_at: new Date().toISOString(),
  }
  render(<Message message={m} />)
  expect(screen.getByText('hello world')).toBeInTheDocument()
})

test('renders assistant bubble with markdown', () => {
  const m: ChatMessage = {
    id: '2',
    role: 'assistant',
    content: '**bold**',
    created_at: new Date().toISOString(),
  }
  const { container } = render(<Message message={m} />)
  expect(container.querySelector('strong')).toHaveTextContent('bold')
})
```

- [ ] **Step 2: Test `ChatInput`**

```tsx
import { render, screen, fireEvent } from '@testing-library/react'
import ChatInput from '../ChatInput'

test('calls onSend with trimmed text and clears', () => {
  const onSend = vi.fn()
  render(<ChatInput onSend={onSend} />)
  const textarea = screen.getByPlaceholderText('Message Plexus…') as HTMLTextAreaElement
  fireEvent.change(textarea, { target: { value: '  hi  ' } })
  fireEvent.keyDown(textarea, { key: 'Enter' })
  expect(onSend).toHaveBeenCalledWith('hi')
  expect(textarea.value).toBe('')
})

test('shift+enter inserts newline instead of sending', () => {
  const onSend = vi.fn()
  render(<ChatInput onSend={onSend} />)
  const textarea = screen.getByPlaceholderText('Message Plexus…') as HTMLTextAreaElement
  fireEvent.change(textarea, { target: { value: 'line 1' } })
  fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: true })
  expect(onSend).not.toHaveBeenCalled()
})
```

- [ ] **Step 3: Test `ProgressHint`**

```tsx
import { render, screen } from '@testing-library/react'
import ProgressHint from '../ProgressHint'

test('renders the hint text', () => {
  render(<ProgressHint text="Executing shell on laptop..." />)
  expect(screen.getByText('Executing shell on laptop...')).toBeInTheDocument()
})
```

- [ ] **Step 4: Run tests**

Run: `cd plexus-frontend && npm test`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add plexus-frontend/src/components/__tests__/
git commit -m "test(frontend): component smoke tests for chat primitives"
```

---

### Task 24: End-to-End Manual Validation + Workspace Integration Check

**Files:** None — this is a validation-only task.

- [ ] **Step 1: Ensure workspace build is clean**

```bash
cd /home/yucheng/Documents/GitHub/Plexus
cargo build
```

Expected: Every crate compiles.

- [ ] **Step 2: Build the frontend**

```bash
cd plexus-frontend
npm ci
npm run build
```

Expected: `plexus-frontend/dist/` contains `index.html` and `assets/`.

- [ ] **Step 3: Configure gateway `.env` to serve the built frontend**

Update `plexus-gateway/.env`:

```bash
PLEXUS_GATEWAY_TOKEN=test-token-12345
JWT_SECRET=<same as plexus-server JWT_SECRET>
GATEWAY_PORT=9090
PLEXUS_SERVER_API_URL=http://localhost:3030
PLEXUS_FRONTEND_DIR=../plexus-frontend/dist
```

- [ ] **Step 4: Run full stack and click through**

In three terminals:

```bash
# Terminal 1
cargo run --package plexus-server
# Terminal 2
cargo run --package plexus-gateway
# Terminal 3 (optional — dev server for hot reload, otherwise skip)
cd plexus-frontend && npm run dev
```

Open http://localhost:9090 in a browser (prod) or http://localhost:5173 (dev).

Manual checklist:

1. Login works with an existing account.
2. Chat page loads; greeting visible with input box mid-screen.
3. Send a message → agent reply appears.
4. Progress hint shows during tool execution (if the agent calls a tool), disappears when the final message arrives.
5. Sidebar sessions can be switched; session history loads.
6. Collapse and expand the sidebar.
7. Device status dots in the top bar reflect `/api/devices` output.
8. Settings page: each tab loads its data. Save soul/memory/devices/channels/skills/cron, verify round-trip.
9. Admin page (if admin): LLM, default soul, rate limit, server MCP all load and save.
10. Logout returns to /login.

- [ ] **Step 5: Commit validation note**

```bash
cd /home/yucheng/Documents/GitHub/Plexus
git commit --allow-empty -m "chore(m3): full-stack validation passed"
```

---

## Self-Review Summary

**Spec coverage:** All sections of the M3 design spec have corresponding tasks:

- Gateway crate layout → Task 1
- Config → Task 2
- JWT validation → Task 3
- State (DashMap + plexus sender) → Task 4
- Static files + SPA fallback → Task 5
- Routing module → Task 6
- `/ws/chat` browser handler → Task 7
- `/ws/plexus` server handler → Task 8
- REST proxy → Task 9
- Integration tests + Postman gate → Task 10
- Frontend scaffold → Task 11
- Types + API wrapper → Task 12
- Auth store + login + router → Task 13
- WebSocket manager → Task 14
- Chat store → Task 15
- Devices store + markdown → Task 16
- Chat components → Task 17
- Chat page wiring → Task 18
- Settings profile → Task 19
- Settings devices → Task 20
- Settings channels/skills/cron → Task 21
- Admin page → Task 22
- Component tests → Task 23
- Full-stack validation → Task 24

**Placeholder check:** No TBDs, no "add appropriate error handling" handwaves, no "similar to Task N" references without code.

**Type consistency:** `BrowserConnection`, `OutboundFrame`, `AppState`, `Claims`, `WsIncoming`, `WsOutgoing`, `ChatMessage` all match across tasks. Zustand stores use consistent selector patterns. API wrapper and all consumers agree on `ApiError` shape.

**Scope check:** Plan is gated between Phase 1 (Task 10) and Phase 2 (Task 11+) with explicit user validation. Both phases ship complete, testable software.
