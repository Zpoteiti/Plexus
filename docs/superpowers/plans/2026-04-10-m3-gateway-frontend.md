# M3: Gateway + Frontend Implementation Plan (r2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `plexus-gateway` (Rust WebSocket hub + REST proxy + static file server) and `plexus-frontend` (React 19 SPA with Chat / Settings / Admin pages).

**Architecture:** The gateway is a stateless "pipe with auth" — browsers and plexus-server both dial in as WebSocket clients, and the gateway routes messages between them by `chat_id`. **The browser owns session state**: every inbound `message` carries a `session_id` that the gateway validates (prefix match against the JWT `sub`) and forwards. Session rows are auto-created on plexus-server on first use (same pattern as Discord/Telegram channels). The frontend is a React SPA styled as "Cyberpunk Refined" (GitHub-dark base + neon green `#39ff14` accents) served by the gateway as static files in production.

**Tech Stack:**
- Gateway: Rust 2024 edition, axum 0.8 (with `ws` feature), jsonwebtoken, subtle, dashmap, reqwest, tower-http, tokio, tokio-util (CancellationToken), dotenvy, tracing.
- Frontend: React 19, TypeScript 5.9, Vite 8, Tailwind CSS 4, Zustand 5, react-router-dom 7, react-markdown + remark-gfm, react-syntax-highlighter, lucide-react, Vitest.

**Spec:** `docs/superpowers/specs/2026-04-10-m3-gateway-frontend-design.md` (r2)
**Protocol:** `plexus-gateway/docs/PROTOCOL.md` (r2)

**Revision r2 highlights:**
- Browser owns session state; gateway validates `session_id` prefix. No more `new_session` / `switch_session` / `session_created` / `session_switched` messages.
- Routing is strictly non-blocking: slow browsers are evicted instead of stalling the plexus reader.
- Writer task lifecycle: explicit drop of local senders before `writer.await`.
- Proxy response body limited to 25 MB via streaming.
- App-level ping/pong on both WS endpoints.
- Graceful shutdown via `CancellationToken`.
- `/healthz` endpoint for load balancers.
- CORS / WS Origin gated on `PLEXUS_ALLOWED_ORIGINS`.
- Frontend WS reconnect has jitter + auth-failed terminal state.
- Frontend URL-driven session (`/chat/:sessionId`); chat store merges REST/WS with dedup.

**Delivery Phases:**
- **Phase 1 (Tasks 1–11):** Gateway. User validates with Postman before Phase 2 starts.
- **Phase 2 (Tasks 12–25):** Frontend.

---

## File Map

### Phase 1 — plexus-gateway

| File | Responsibility |
|---|---|
| `Cargo.toml` (workspace) | Add `plexus-gateway` member |
| `plexus-gateway/Cargo.toml` | Crate deps |
| `plexus-gateway/.env.example` | Env var template |
| `plexus-gateway/src/main.rs` | Entry point, signal handlers, serve |
| `plexus-gateway/src/lib.rs` | Router builder, `serve()`, `run_from_env()` |
| `plexus-gateway/src/config.rs` | `Config` struct with `allowed_origins: Vec<String>` |
| `plexus-gateway/src/state.rs` | `AppState` with DashMap, plexus sender, shutdown token |
| `plexus-gateway/src/jwt.rs` | JWT validation using `jsonwebtoken` |
| `plexus-gateway/src/routing.rs` | Non-blocking chat_id lookup with eviction on queue full |
| `plexus-gateway/src/proxy.rs` | REST `/api/*` reverse proxy with streamed response cap |
| `plexus-gateway/src/static_files.rs` | Frontend static file serving with SPA fallback |
| `plexus-gateway/src/health.rs` | `/healthz` handler |
| `plexus-gateway/src/ws/mod.rs` | Shared WS types, `BrowserConnection`, `OutboundFrame` |
| `plexus-gateway/src/ws/chat.rs` | `/ws/chat` browser handler (ping/pong, session prefix check, lifecycle) |
| `plexus-gateway/src/ws/plexus.rs` | `/ws/plexus` plexus-server handler (ping/pong, graceful shutdown) |
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
tokio-util = "0.7"  # CancellationToken for graceful shutdown
tower-http = { version = "0.6", features = ["cors", "fs", "trace", "limit"] }
futures-util = "0.3"
jsonwebtoken = "9"
subtle = "2"
dashmap = "6"
reqwest = { version = "0.12", features = ["json", "stream"] }
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"

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

# Comma-separated CORS and WebSocket Origin allow-list.
# Use "*" for local dev only. Production MUST set an explicit list.
# Example: PLEXUS_ALLOWED_ORIGINS=https://plexus.example.com,https://admin.plexus.example.com
PLEXUS_ALLOWED_ORIGINS=*
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
    /// Comma-separated list from PLEXUS_ALLOWED_ORIGINS. Empty = wildcard (dev only).
    pub allowed_origins: Vec<String>,
}

impl Config {
    /// Load from environment variables. Panics if any required var is missing.
    pub fn from_env() -> Self {
        let raw_origins = env::var("PLEXUS_ALLOWED_ORIGINS").unwrap_or_else(|_| "*".to_string());
        let allowed_origins = if raw_origins.trim() == "*" {
            Vec::new() // empty = wildcard
        } else {
            raw_origins
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        Self {
            gateway_token: required("PLEXUS_GATEWAY_TOKEN"),
            jwt_secret: required("JWT_SECRET"),
            port: required("GATEWAY_PORT")
                .parse()
                .expect("GATEWAY_PORT must be a valid u16"),
            plexus_server_api_url: required("PLEXUS_SERVER_API_URL"),
            frontend_dir: env::var("PLEXUS_FRONTEND_DIR")
                .unwrap_or_else(|_| "../plexus-frontend/dist".to_string()),
            allowed_origins,
        }
    }

    /// Returns true if origin is allowed. Empty allow-list = wildcard.
    pub fn origin_allowed(&self, origin: Option<&str>) -> bool {
        if self.allowed_origins.is_empty() {
            return true;
        }
        match origin {
            Some(o) => self.allowed_origins.iter().any(|allowed| allowed == o),
            None => false,
        }
    }
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("Required env var {name} is not set"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_allows_any_origin() {
        let c = Config {
            gateway_token: "t".into(),
            jwt_secret: "s".into(),
            port: 0,
            plexus_server_api_url: "http://localhost".into(),
            frontend_dir: "/tmp".into(),
            allowed_origins: vec![],
        };
        assert!(c.origin_allowed(Some("https://example.com")));
        assert!(c.origin_allowed(None));
    }

    #[test]
    fn strict_list_rejects_others() {
        let c = Config {
            gateway_token: "t".into(),
            jwt_secret: "s".into(),
            port: 0,
            plexus_server_api_url: "http://localhost".into(),
            frontend_dir: "/tmp".into(),
            allowed_origins: vec!["https://plexus.example.com".into()],
        };
        assert!(c.origin_allowed(Some("https://plexus.example.com")));
        assert!(!c.origin_allowed(Some("https://evil.example.com")));
        assert!(!c.origin_allowed(None));
    }
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

- [ ] **Step 2: Declare `jwt` module in `main.rs`**

Add to the module list in `plexus-gateway/src/main.rs`:

```rust
mod config;
mod jwt;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --package plexus-gateway jwt::`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/jwt.rs plexus-gateway/src/main.rs
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

/// A frame queued for delivery to a browser.
///
/// Lifecycle:
/// - Progress frames are dropped silently on channel full (ephemeral hints).
/// - Message frames trigger eviction on channel full (slow consumer protection).
/// - Close frames are enqueued during graceful shutdown so the writer flushes
///   and closes the sink cleanly.
#[derive(Debug, Clone)]
pub enum OutboundFrame {
    Message(Value),
    Progress(Value),
    Close,
}

/// Per-browser handle held in `AppState.browsers`. Cloneable so the routing
/// layer can clone the handle out of the DashMap shard before any await.
///
/// The WebSocket sink is not stored here — it's owned by a dedicated writer
/// task. Other tasks send frames through the bounded `outbound` channel.
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
use tokio_util::sync::CancellationToken;

pub struct AppState {
    pub config: Config,
    /// chat_id → cloneable browser handle
    pub browsers: Arc<DashMap<String, BrowserConnection>>,
    /// Sender for the single plexus-server connection. `None` when not connected.
    pub plexus: Arc<RwLock<Option<mpsc::Sender<Value>>>>,
    /// Pooled HTTP client for REST proxy requests.
    pub http_client: reqwest::Client,
    /// Triggered on SIGTERM/SIGINT. Reader loops watch this and break cleanly.
    pub shutdown: CancellationToken,
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
            shutdown: CancellationToken::new(),
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
//! Guarantees:
//! 1. `route_send` is **non-blocking**. It uses `try_send` on the per-browser
//!    outbound channel and never awaits on slow consumers. This is critical
//!    because route_send is called from the single /ws/plexus reader loop
//!    and blocking here head-of-line-blocks every browser.
//! 2. DashMap shard guards are **never** held across await points. Handles
//!    are cloned out synchronously before any async work.
//! 3. On queue-full for a final Message, the browser is **evicted** from
//!    state.browsers. Its writer task exits once the last sender is dropped.
//! 4. On queue-full for a Progress frame, the frame is **dropped silently**.
//!    Progress is ephemeral.

use crate::state::AppState;
use crate::ws::{BrowserConnection, OutboundFrame};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, PartialEq, Eq)]
pub enum RouteResult {
    /// Delivered via direct chat_id lookup.
    DirectHit,
    /// Delivered via sender_id fallback.
    SenderFallback,
    /// No matching browser connection found.
    NoMatch,
    /// A browser was evicted due to queue pressure (final message).
    Evicted,
}

/// Build an outbound JSON frame from an upstream `send` message.
/// Returns (frame, chat_id, sender_id_opt).
fn build_frame(msg: &Value) -> (OutboundFrame, String, Option<String>) {
    let chat_id = msg
        .get("chat_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let content = msg.get("content").cloned().unwrap_or(Value::Null);
    let session_id = msg.get("session_id").cloned().unwrap_or(Value::Null);
    let metadata = msg.get("metadata").cloned().unwrap_or(json!({}));

    let is_progress = metadata
        .get("_progress")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let media = metadata.get("media").cloned();
    let sender_id = metadata
        .get("sender_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let frame_type = if is_progress { "progress" } else { "message" };
    let mut outbound = json!({
        "type": frame_type,
        "session_id": session_id,
        "content": content,
    });
    if let Some(media) = media {
        outbound["media"] = media;
    }

    let frame = if is_progress {
        OutboundFrame::Progress(outbound)
    } else {
        OutboundFrame::Message(outbound)
    };

    (frame, chat_id, sender_id)
}

/// Route a `send` message from plexus-server to the correct browser.
/// Non-blocking — never awaits on a browser's outbound channel.
pub fn route_send(state: &Arc<AppState>, msg: &Value) -> RouteResult {
    let (frame, chat_id, sender_id) = build_frame(msg);

    // Direct lookup — clone the handle OUT of the shard before any further work.
    let direct_conn = state.browsers.get(&chat_id).map(|r| r.clone());
    if let Some(conn) = direct_conn {
        match try_dispatch(state, &chat_id, conn, frame.clone()) {
            DispatchOutcome::Delivered => return RouteResult::DirectHit,
            DispatchOutcome::Dropped => return RouteResult::DirectHit,
            DispatchOutcome::Evicted => return RouteResult::Evicted,
        }
    }

    // Fallback: any browser for the given sender_id.
    if let Some(sender_id) = sender_id {
        // Snapshot matching handles. This iterates under the DashMap read
        // lock but does not await; clones are cheap (Arc + String).
        let candidates: Vec<(String, BrowserConnection)> = state
            .browsers
            .iter()
            .filter(|entry| entry.value().user_id == sender_id)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        for (fallback_chat_id, conn) in candidates {
            match try_dispatch(state, &fallback_chat_id, conn, frame.clone()) {
                DispatchOutcome::Delivered => return RouteResult::SenderFallback,
                DispatchOutcome::Dropped => return RouteResult::SenderFallback,
                DispatchOutcome::Evicted => continue, // try the next candidate
            }
        }
    }

    warn!("routing: no match for chat_id={chat_id}");
    RouteResult::NoMatch
}

enum DispatchOutcome {
    Delivered,
    Dropped,  // progress frame dropped on full
    Evicted,  // final frame could not be delivered; browser evicted
}

fn try_dispatch(
    state: &Arc<AppState>,
    chat_id: &str,
    conn: BrowserConnection,
    frame: OutboundFrame,
) -> DispatchOutcome {
    match &frame {
        OutboundFrame::Progress(_) => match conn.outbound.try_send(frame) {
            Ok(()) => DispatchOutcome::Delivered,
            Err(_) => DispatchOutcome::Dropped,
        },
        OutboundFrame::Message(_) => match conn.outbound.try_send(frame) {
            Ok(()) => DispatchOutcome::Delivered,
            Err(_) => {
                warn!("routing: evicting slow browser chat_id={chat_id}");
                state.browsers.remove(chat_id);
                DispatchOutcome::Evicted
            }
        },
        OutboundFrame::Close => {
            // Close is only sent from shutdown path, not routing.
            let _ = conn.outbound.try_send(frame);
            DispatchOutcome::Delivered
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
            allowed_origins: vec![],
        };
        AppState::new(config)
    }

    fn register_browser(
        state: &Arc<AppState>,
        chat_id: &str,
        user_id: &str,
        buffer: usize,
    ) -> mpsc::Receiver<OutboundFrame> {
        let (tx, rx) = mpsc::channel(buffer);
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
        let mut rx = register_browser(&state, "chat-1", "user-1", 8);
        let msg = json!({
            "type": "send",
            "chat_id": "chat-1",
            "session_id": "gateway:user-1:abc",
            "content": "hello",
        });
        assert_eq!(route_send(&state, &msg), RouteResult::DirectHit);
        match rx.recv().await.unwrap() {
            OutboundFrame::Message(v) => {
                assert_eq!(v["type"], "message");
                assert_eq!(v["content"], "hello");
                assert_eq!(v["session_id"], "gateway:user-1:abc");
            }
            _ => panic!("expected Message"),
        }
    }

    #[tokio::test]
    async fn sender_id_fallback() {
        let state = test_state();
        let mut rx = register_browser(&state, "chat-existing", "user-42", 8);
        let msg = json!({
            "type": "send",
            "chat_id": "chat-stale",
            "session_id": "gateway:user-42:xyz",
            "content": "scheduled task result",
            "metadata": { "sender_id": "user-42" },
        });
        assert_eq!(route_send(&state, &msg), RouteResult::SenderFallback);
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
        assert_eq!(route_send(&state, &msg), RouteResult::NoMatch);
    }

    #[tokio::test]
    async fn progress_frame_sets_type() {
        let state = test_state();
        let mut rx = register_browser(&state, "chat-p", "user-p", 8);
        let msg = json!({
            "type": "send",
            "chat_id": "chat-p",
            "session_id": "gateway:user-p:abc",
            "content": "Executing shell on laptop...",
            "metadata": { "_progress": true },
        });
        assert_eq!(route_send(&state, &msg), RouteResult::DirectHit);
        match rx.recv().await.unwrap() {
            OutboundFrame::Progress(v) => {
                assert_eq!(v["type"], "progress");
                assert_eq!(v["content"], "Executing shell on laptop...");
            }
            _ => panic!("expected Progress"),
        }
    }

    #[tokio::test]
    async fn slow_browser_evicted_on_final_message() {
        let state = test_state();
        // Buffer of 1; don't drain. First send fills the queue.
        let _rx = register_browser(&state, "chat-slow", "user-slow", 1);
        let filler = json!({
            "type": "send",
            "chat_id": "chat-slow",
            "session_id": "gateway:user-slow:abc",
            "content": "first",
        });
        assert_eq!(route_send(&state, &filler), RouteResult::DirectHit);
        // Second final send — queue full → Evicted.
        let full = json!({
            "type": "send",
            "chat_id": "chat-slow",
            "session_id": "gateway:user-slow:abc",
            "content": "second",
        });
        assert_eq!(route_send(&state, &full), RouteResult::Evicted);
        // Browser is removed from state.browsers.
        assert!(!state.browsers.contains_key("chat-slow"));
    }

    #[tokio::test]
    async fn progress_frame_dropped_silently_on_full() {
        let state = test_state();
        // Buffer of 1; don't drain.
        let _rx = register_browser(&state, "chat-p2", "user-p2", 1);
        let first = json!({
            "type": "send",
            "chat_id": "chat-p2",
            "session_id": "gateway:user-p2:abc",
            "content": "first progress",
            "metadata": { "_progress": true },
        });
        route_send(&state, &first);
        // Second one hits a full queue — dropped, but NOT evicted.
        let second = json!({
            "type": "send",
            "chat_id": "chat-p2",
            "session_id": "gateway:user-p2:abc",
            "content": "second progress",
            "metadata": { "_progress": true },
        });
        let result = route_send(&state, &second);
        assert_eq!(result, RouteResult::DirectHit); // it was "delivered" (dropped counts)
        // Crucially, browser is still registered.
        assert!(state.browsers.contains_key("chat-p2"));
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
Expected: 6 tests pass (direct_chat_id_hit, sender_id_fallback, no_match_returns_nomatch, progress_frame_sets_type, slow_browser_evicted_on_final_message, progress_frame_dropped_silently_on_full).

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/routing.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): non-blocking routing with slow-browser eviction"
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
//! Protocol (r2):
//! - Browser owns session state. Every inbound `message` carries a `session_id`.
//! - Gateway validates the prefix matches `gateway:{user_id}:` against JWT sub.
//! - No session_created / new_session / switch_session messages.
//!
//! Lifecycle:
//! - Origin check → JWT validation → upgrade.
//! - Spawn writer task owning the sink; reader loop forwards to plexus via
//!   bounded mpsc channel.
//! - Spawn keepalive task: app-level ping every 30s, expect pong within 15s.
//! - Shutdown (via CancellationToken) drains gracefully.
//! - On disconnect: remove from state.browsers, drop local sender, await writer.

use crate::jwt;
use crate::state::AppState;
use crate::ws::{BrowserConnection, OutboundFrame};
use axum::extract::{
    ConnectInfo, Query, State, WebSocketUpgrade,
    ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code},
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, interval};
use tracing::{info, info_span, warn, Instrument};

const PING_INTERVAL: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(15);
const OUTBOUND_BUFFER: usize = 64;

#[derive(Deserialize)]
pub struct ChatQuery {
    pub token: String,
}

pub async fn handler(
    ws: WebSocketUpgrade,
    Query(params): Query<ChatQuery>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
) -> Response {
    // Origin check (relies on Config::origin_allowed: empty list = wildcard)
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    if !state.config.origin_allowed(origin) {
        warn!("ws/chat: rejected origin {:?}", origin);
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }

    // JWT validation
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
    let span = info_span!("ws_chat", chat_id = %chat_id, user_id = %user_id);
    run_inner(socket, state, user_id, chat_id).instrument(span).await;
}

async fn run_inner(socket: WebSocket, state: Arc<AppState>, user_id: String, chat_id: String) {
    let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundFrame>(OUTBOUND_BUFFER);

    state.browsers.insert(
        chat_id.clone(),
        BrowserConnection {
            outbound: outbound_tx.clone(),
            user_id: user_id.clone(),
        },
    );
    info!("browser connected");

    let (ws_sink, mut ws_stream) = socket.split();

    // Last-pong timestamp, updated by the reader loop, checked by keepalive.
    let last_pong = Arc::new(AtomicI64::new(now_ms()));

    // Writer task
    let writer = tokio::spawn(writer_task(ws_sink, outbound_rx));

    // Keepalive task
    let keepalive = tokio::spawn(keepalive_task(
        outbound_tx.clone(),
        Arc::clone(&last_pong),
    ));

    // Expected session_id prefix for this connection.
    let session_prefix = format!("gateway:{}:", user_id);

    let shutdown = state.shutdown.clone();

    // Reader loop
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("shutdown signal received; closing browser");
                let _ = outbound_tx.send(OutboundFrame::Close).await;
                break;
            }
            msg = ws_stream.next() => {
                let Some(Ok(msg)) = msg else { break };
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
                        handle_message(&state, &chat_id, &user_id, &session_prefix, &parsed, &outbound_tx).await;
                    }
                    "pong" => {
                        last_pong.store(now_ms(), Ordering::Relaxed);
                    }
                    other => {
                        warn!("unknown message type: {other}");
                    }
                }
            }
        }
    }

    // Cleanup — order matters for clean writer exit.
    cleanup(&state, &chat_id).await;
    drop(outbound_tx); // drop the local sender so the writer can finish draining
    let _ = writer.await;
    keepalive.abort();
    info!("browser disconnected");
}

async fn handle_message(
    state: &Arc<AppState>,
    chat_id: &str,
    user_id: &str,
    session_prefix: &str,
    parsed: &Value,
    outbound_tx: &mpsc::Sender<OutboundFrame>,
) {
    let content = parsed
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let media = parsed.get("media").cloned().unwrap_or(Value::Null);
    let session_id = parsed
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Validate session_id prefix — prevents cross-user spoofing.
    if session_id.is_empty() || !session_id.starts_with(session_prefix) {
        warn!(
            "rejecting message with invalid session_id prefix: got {:?}, expected prefix {:?}",
            session_id, session_prefix
        );
        let _ = outbound_tx
            .try_send(OutboundFrame::Message(json!({
                "type": "error",
                "reason": "invalid session_id",
            })));
        return;
    }

    // Look up plexus sender (clone the handle, drop the read guard immediately).
    let plexus_tx = {
        let guard = state.plexus.read().await;
        guard.clone()
    };
    let Some(plexus_tx) = plexus_tx else {
        let _ = outbound_tx
            .try_send(OutboundFrame::Message(json!({
                "type": "error",
                "reason": "Plexus server not connected",
            })));
        return;
    };

    let mut payload = json!({
        "type": "message",
        "chat_id": chat_id,
        "sender_id": user_id,
        "session_id": session_id,
        "content": content,
    });
    if !media.is_null() {
        payload["media"] = media;
    }
    if plexus_tx.send(payload).await.is_err() {
        warn!("plexus channel closed while forwarding");
    }
}

async fn writer_task(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<OutboundFrame>,
) {
    while let Some(frame) = rx.recv().await {
        match frame {
            OutboundFrame::Message(v) | OutboundFrame::Progress(v) => {
                let text = serde_json::to_string(&v).unwrap_or_default();
                if sink.send(Message::Text(Utf8Bytes::from(text))).await.is_err() {
                    break;
                }
            }
            OutboundFrame::Close => {
                let _ = sink
                    .send(Message::Close(Some(CloseFrame {
                        code: close_code::AWAY,
                        reason: Utf8Bytes::from("server shutting down"),
                    })))
                    .await;
                break;
            }
        }
    }
}

async fn keepalive_task(
    outbound: mpsc::Sender<OutboundFrame>,
    last_pong: Arc<AtomicI64>,
) {
    let mut tick = interval(PING_INTERVAL);
    tick.tick().await; // first tick fires immediately; skip it
    loop {
        tick.tick().await;
        // Send ping
        let ping = json!({"type": "ping"});
        if outbound
            .send(OutboundFrame::Message(ping))
            .await
            .is_err()
        {
            return;
        }
        // Wait briefly and check for pong within PONG_TIMEOUT
        let deadline = Instant::now() + PONG_TIMEOUT;
        let sent_at = now_ms();
        loop {
            tokio::time::sleep_until(deadline.min(Instant::now() + Duration::from_millis(500))).await;
            if last_pong.load(Ordering::Relaxed) >= sent_at {
                break; // got a pong
            }
            if Instant::now() >= deadline {
                warn!("keepalive: no pong within {PONG_TIMEOUT:?}, closing");
                // Send Close; writer will exit; reader will terminate.
                let _ = outbound.try_send(OutboundFrame::Close);
                return;
            }
        }
    }
}

async fn cleanup(state: &Arc<AppState>, chat_id: &str) {
    state.browsers.remove(chat_id);
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
```

- [ ] **Step 2: Wire `/ws/chat` into the router in `main.rs`**

`ConnectInfo<SocketAddr>` extraction requires the listener to be served with `.into_make_service_with_connect_info::<SocketAddr>()`. Update `plexus-gateway/src/main.rs`:

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
use std::net::SocketAddr;
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
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
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
//! 5. Writer task owns the sink. Reader loop routes `send` messages via
//!    non-blocking `routing::route_send`.
//! 6. Shutdown signal breaks the reader loop; cleanup drops the plexus sender
//!    (which ends the writer's recv()) and awaits the writer join handle.

use crate::routing;
use crate::state::AppState;
use axum::extract::{
    State, WebSocketUpgrade,
    ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code},
};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tracing::{info, info_span, warn, Instrument};

const PLEXUS_BUFFER: usize = 256;

pub async fn handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| run(socket, state).instrument(info_span!("ws_plexus")))
}

async fn run(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Step 1: wait for auth frame (5s)
    let auth_msg = match timeout(Duration::from_secs(5), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => t,
        _ => {
            let _ = send_text(&mut sink, json!({"type":"auth_fail","reason":"no auth"})).await;
            return;
        }
    };

    let parsed: Value = match serde_json::from_str(&auth_msg) {
        Ok(v) => v,
        Err(_) => {
            let _ = send_text(&mut sink, json!({"type":"auth_fail","reason":"invalid json"})).await;
            return;
        }
    };

    if parsed.get("type").and_then(|v| v.as_str()) != Some("auth") {
        let _ = send_text(&mut sink, json!({"type":"auth_fail","reason":"expected auth"})).await;
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
        let _ = send_text(&mut sink, json!({"type":"auth_fail","reason":"invalid token"})).await;
        return;
    }

    // Step 2: enforce singleton and create the sender channel.
    let (plexus_tx, mut plexus_rx) = mpsc::channel::<Value>(PLEXUS_BUFFER);
    {
        let mut guard = state.plexus.write().await;
        if guard.is_some() {
            let _ = send_text(&mut sink, json!({"type":"auth_fail","reason":"duplicate connection"})).await;
            return;
        }
        *guard = Some(plexus_tx);
    }

    // Step 3: ack
    if send_text(&mut sink, json!({"type":"auth_ok"})).await.is_err() {
        state.plexus.write().await.take();
        return;
    }
    info!("plexus server authenticated");

    // Writer task: drain plexus_rx into the sink. Exits when all senders drop.
    let writer = tokio::spawn(async move {
        while let Some(value) = plexus_rx.recv().await {
            let text = serde_json::to_string(&value).unwrap_or_default();
            if sink.send(Message::Text(Utf8Bytes::from(text))).await.is_err() {
                break;
            }
        }
        // Try to send a close frame on the way out.
        let _ = sink
            .send(Message::Close(Some(CloseFrame {
                code: close_code::NORMAL,
                reason: Utf8Bytes::from("gateway closing"),
            })))
            .await;
    });

    // Reader loop: route `send` messages (non-blocking), honor shutdown signal.
    let shutdown = state.shutdown.clone();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("shutdown signal received; closing plexus connection");
                break;
            }
            msg = stream.next() => {
                let Some(Ok(msg)) = msg else { break };
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
                    "send" => {
                        let _ = routing::route_send(&state, &parsed);
                    }
                    "pong" => {
                        // plexus is alive — no action needed
                    }
                    other => {
                        warn!("unknown message type: {other}");
                    }
                }
            }
        }
    }

    // Cleanup: dropping the sender stored in state.plexus causes the writer's
    // recv() to return None and the writer task exits cleanly.
    state.plexus.write().await.take();
    let _ = writer.await;
    info!("plexus server disconnected");
}

async fn send_text(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    value: Value,
) -> Result<(), axum::Error> {
    let text = value.to_string();
    sink.send(Message::Text(Utf8Bytes::from(text))).await
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
//! - Max request body: 25 MB (enforced by tower-http RequestBodyLimitLayer).
//! - Max response body: 25 MB (enforced here by streaming with a running counter
//!   plus a Content-Length fast path).
//! - Hop-by-hop headers stripped.
//! - Path traversal rejected.

use crate::jwt;
use crate::state::AppState;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use futures_util::StreamExt;
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
            error_json("path_traversal", "path traversal rejected"),
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
            return (
                StatusCode::UNAUTHORIZED,
                error_json("unauthorized", "invalid or missing token"),
            )
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
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                error_json("request_too_large", "request body exceeded 25 MB limit"),
            )
                .into_response();
        }
    };

    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                error_json("invalid_method", "invalid HTTP method"),
            )
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
                error_json("upstream_unreachable", &format!("upstream unreachable: {e}")),
            )
                .into_response();
        }
    };

    // Content-Length fast path: reject oversized responses before reading body.
    if let Some(cl) = resp.content_length() {
        if cl > MAX_BODY_BYTES as u64 {
            warn!("proxy: upstream response Content-Length {cl} > {MAX_BODY_BYTES}");
            return (
                StatusCode::BAD_GATEWAY,
                error_json(
                    "upstream_too_large",
                    "response body exceeded 25 MB limit",
                ),
            )
                .into_response();
        }
    }

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

    // Streaming body with running size check (handles chunked / missing Content-Length).
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                warn!("proxy: upstream read error: {e}");
                return (
                    StatusCode::BAD_GATEWAY,
                    error_json("upstream_read_error", &format!("read upstream body: {e}")),
                )
                    .into_response();
            }
        };
        if buf.len() + chunk.len() > MAX_BODY_BYTES {
            warn!("proxy: upstream stream exceeded MAX_BODY_BYTES");
            return (
                StatusCode::BAD_GATEWAY,
                error_json("upstream_too_large", "response body exceeded 25 MB limit"),
            )
                .into_response();
        }
        buf.extend_from_slice(&chunk);
    }

    let mut response = Response::new(Body::from(buf));
    *response.status_mut() = status;
    *response.headers_mut() = out_headers;
    response
}

fn error_json(code: &str, message: &str) -> String {
    json!({
        "error": {
            "code": code,
            "message": message,
        }
    })
    .to_string()
}
```

- [ ] **Step 2: Wire proxy into the router in `main.rs`**

This step replaces the router bootstrap with one that uses the CORS policy from `Config`, wires in `/healthz`, and prepares for graceful shutdown (which Task 11 completes). Add proxy + healthz routes in `plexus-gateway/src/main.rs`:

```rust
mod config;
mod health;
mod jwt;
mod proxy;
mod routing;
mod state;
mod static_files;
mod ws;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::routing::get;
use config::Config;
use state::AppState;
use std::net::SocketAddr;
use std::sync::Arc;
use std::str::FromStr;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

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

    let cors = build_cors(&config);

    let app: Router = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/ws/chat", get(ws::chat::handler))
        .route("/ws/plexus", get(ws::plexus::handler))
        .merge(proxy::routes())
        .merge(static_files::service(&config.frontend_dir))
        .layer(cors)
        .with_state(Arc::clone(&state));

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("plexus-gateway listening on {addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

fn build_cors(config: &Config) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-requested-with"),
        ]);

    if config.allowed_origins.is_empty() {
        base.allow_origin(Any)
    } else {
        let list: Vec<HeaderValue> = config
            .allowed_origins
            .iter()
            .filter_map(|s| HeaderValue::from_str(s).ok())
            .collect();
        base.allow_origin(AllowOrigin::list(list))
    }
}
```

Create the stub `plexus-gateway/src/health.rs`:

```rust
//! Unauthenticated /healthz endpoint for load-balancer readiness probes.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use std::sync::Arc;

pub async fn healthz(State(state): State<Arc<AppState>>) -> Json<Value> {
    let plexus_connected = state.plexus.read().await.is_some();
    Json(json!({
        "status": "ok",
        "plexus_connected": plexus_connected,
        "browsers": state.browsers.len(),
    }))
}
```

- [ ] **Step 3: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build (may warn about unused imports in proxy.rs — those are fine).

- [ ] **Step 4: Commit**

```bash
git add plexus-gateway/src/proxy.rs plexus-gateway/src/health.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): add REST reverse proxy, /healthz, and strict CORS"
```

---

### Task 10: Library Split + Graceful Shutdown

**Files:**
- Create: `plexus-gateway/src/lib.rs`
- Modify: `plexus-gateway/src/main.rs`

This task splits the binary so integration tests can import it, wires in graceful shutdown, and sets up the `run_with_config` entry point that tests use.

- [ ] **Step 1: Create `plexus-gateway/src/lib.rs`**

```rust
//! plexus-gateway library entry point. `main.rs` delegates to
//! `run_from_env`. Integration tests use `serve` to spawn isolated instances
//! with a custom `Config` (no env var races).

pub mod config;
pub mod health;
pub mod jwt;
pub mod proxy;
pub mod routing;
pub mod state;
pub mod static_files;
pub mod ws;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::routing::get;
use config::Config;
use state::AppState;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tokio::net::TcpListener;

/// Build the axum router. Exposed for tests.
pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = build_cors(&state.config);

    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/ws/chat", get(ws::chat::handler))
        .route("/ws/plexus", get(ws::plexus::handler))
        .merge(proxy::routes())
        .merge(static_files::service(&state.config.frontend_dir))
        .layer(cors)
        .with_state(state)
}

fn build_cors(config: &Config) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-requested-with"),
        ]);

    if config.allowed_origins.is_empty() {
        base.allow_origin(Any)
    } else {
        let list: Vec<HeaderValue> = config
            .allowed_origins
            .iter()
            .filter_map(|s| HeaderValue::from_str(s).ok())
            .collect();
        base.allow_origin(AllowOrigin::list(list))
    }
}

/// Run the gateway on the given listener with the given config. Honors
/// the state's `shutdown` CancellationToken for graceful shutdown. Used
/// by tests (which pass an ephemeral listener) and by `run_from_env`.
pub async fn serve(listener: TcpListener, config: Config) {
    let state = AppState::new(config);
    let app = build_router(Arc::clone(&state));

    let shutdown = state.shutdown.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown.cancelled().await;
        tracing::info!("graceful shutdown signal received");
        // Give in-flight WS handlers up to 5s to drain after cancellation.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    })
    .await
    .unwrap();
}

/// Run the gateway using env vars for config. Used by `main.rs`.
pub async fn run_from_env() {
    let config = Config::from_env();
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("plexus-gateway listening on {addr}");
    serve(listener, config).await;
}
```

- [ ] **Step 2: Trim `main.rs` to delegate and install signal handlers**

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

    // The state's shutdown token is triggered by install_signal_handlers,
    // which watches SIGTERM and SIGINT. run_from_env sets up the token and
    // wires it into axum's graceful shutdown future via serve().
    //
    // On Unix, tokio::signal supports SIGTERM directly. On Windows, only
    // Ctrl+C is supported.
    let shutdown_task = tokio::spawn(install_signal_handlers());
    plexus_gateway::run_from_env().await;
    shutdown_task.abort();
}

#[cfg(unix)]
async fn install_signal_handlers() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT");
    tokio::select! {
        _ = term.recv() => tracing::info!("SIGTERM received"),
        _ = int.recv() => tracing::info!("SIGINT received"),
    }
    // Trigger shutdown via the global (static) token is not accessible here
    // because AppState is constructed inside serve(). Workaround: signal the
    // process by panic-propagating through the ctrl_c handler path.
    //
    // NOTE: This is tricky because main.rs can't reach into the AppState
    // created by run_from_env. The cleanest fix is to have run_from_env
    // expose the CancellationToken. See Step 3.
}
```

**Note:** the above comment about the signal-handler race is the first draft. Step 3 fixes it by changing `serve` to accept an externally-constructed `AppState` so `main.rs` can hold the token.

- [ ] **Step 3: Refactor `serve` and `run_from_env` to expose the shutdown token**

Replace the previous `lib.rs` `serve`/`run_from_env` + `main.rs` glue with this cleaner version. Update `plexus-gateway/src/lib.rs`:

```rust
// Replace just the serve + run_from_env functions at the bottom of lib.rs:

/// Run the gateway on the given listener with the given pre-built state.
/// The caller owns the `AppState.shutdown` token and is responsible for
/// triggering it (e.g. from a signal handler).
pub async fn serve_with_state(listener: TcpListener, state: Arc<AppState>) {
    let app = build_router(Arc::clone(&state));
    let shutdown = state.shutdown.clone();

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown.cancelled().await;
        tracing::info!("graceful shutdown signal received");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    })
    .await
    .unwrap();
}

/// Convenience: build state from config and call serve_with_state. Tests use this.
pub async fn serve(listener: TcpListener, config: Config) {
    let state = AppState::new(config);
    serve_with_state(listener, state).await;
}

/// Run the gateway using env vars for config. Used by `main.rs`. Returns
/// the shutdown token so main can trigger it from a signal handler.
pub async fn run_from_env() {
    let config = Config::from_env();
    let state = AppState::new(config);
    let shutdown = state.shutdown.clone();

    // Install signal handlers that fire the shutdown token.
    tokio::spawn(wait_for_signal(shutdown));

    let addr = format!("0.0.0.0:{}", state.config.port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("plexus-gateway listening on {addr}");
    serve_with_state(listener, state).await;
}

#[cfg(unix)]
async fn wait_for_signal(shutdown: tokio_util::sync::CancellationToken) {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT");
    tokio::select! {
        _ = term.recv() => tracing::info!("SIGTERM received; beginning graceful shutdown"),
        _ = int.recv() => tracing::info!("SIGINT received; beginning graceful shutdown"),
    }
    shutdown.cancel();
}

#[cfg(not(unix))]
async fn wait_for_signal(shutdown: tokio_util::sync::CancellationToken) {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Ctrl-C received; beginning graceful shutdown");
    shutdown.cancel();
}
```

And simplify `plexus-gateway/src/main.rs`:

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

(The main.rs from Task 9 Step 2 — with all the inline router setup — is now obsolete because `build_router` + `serve_with_state` live in lib.rs. Delete the duplicate modules section from main.rs.)

- [ ] **Step 4: Build**

Run: `cargo build --package plexus-gateway`
Expected: Clean build.

- [ ] **Step 5: Commit**

```bash
git add plexus-gateway/src/lib.rs plexus-gateway/src/main.rs
git commit -m "feat(gateway): library split with graceful shutdown"
```

---

### Task 11: Integration Tests + Postman Validation Gate

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
use plexus_gateway::state::AppState;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

const JWT_SECRET: &str = "test-jwt-secret";
const GATEWAY_TOKEN: &str = "test-gateway-token";

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

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

fn test_config(port: u16, upstream: &str, frontend_dir: &str) -> Config {
    Config {
        gateway_token: GATEWAY_TOKEN.to_string(),
        jwt_secret: JWT_SECRET.to_string(),
        port,
        plexus_server_api_url: upstream.to_string(),
        frontend_dir: frontend_dir.to_string(),
        allowed_origins: vec![], // wildcard for tests
    }
}

/// Start a gateway instance in-process on an ephemeral port. Returns the
/// state handle (so tests can trigger shutdown) and the port.
async fn spawn_gateway_with(config: Config) -> (Arc<AppState>, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let state = AppState::new(Config { port, ..config });
    let state_for_serve = Arc::clone(&state);
    tokio::spawn(async move {
        plexus_gateway::serve_with_state(listener, state_for_serve).await;
    });

    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return (state, port);
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("gateway failed to bind within ~1s");
}

async fn spawn_gateway() -> (Arc<AppState>, u16) {
    spawn_gateway_with(test_config(0, "http://127.0.0.1:1", "/tmp")).await
}

async fn session_id_for(user: &str) -> String {
    format!("gateway:{}:{}", user, uuid::Uuid::new_v4())
}

async fn connect_plexus(port: u16) -> WsStream {
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
    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        WsMessage::Text(t) => t,
        _ => panic!("expected text"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "auth_ok");
    ws
}

async fn connect_browser(port: u16, user: &str) -> WsStream {
    let jwt = valid_jwt(user);
    let (ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}"))
        .await
        .unwrap();
    ws
}

/// Drain any asynchronous ping frames the gateway may have sent before
/// the real test assertion. Returns the first non-ping Text frame.
async fn recv_non_ping(ws: &mut WsStream) -> Value {
    loop {
        let msg = ws.next().await.unwrap().unwrap();
        let text = match msg {
            WsMessage::Text(t) => t,
            WsMessage::Close(_) => panic!("unexpected close"),
            _ => continue,
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        if v["type"] == "ping" {
            // Reply with pong to keep the connection healthy.
            ws.send(WsMessage::Text(json!({"type":"pong"}).to_string().into()))
                .await
                .unwrap();
            continue;
        }
        return v;
    }
}

#[tokio::test]
async fn browser_to_plexus_round_trip() {
    let (_state, port) = spawn_gateway().await;
    let mut plexus = connect_plexus(port).await;
    let mut browser = connect_browser(port, "user-1").await;

    let sid = session_id_for("user-1").await;
    browser
        .send(WsMessage::Text(
            json!({"type":"message","session_id":sid,"content":"hello"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let v = recv_non_ping(&mut plexus).await;
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "hello");
    assert_eq!(v["sender_id"], "user-1");
    assert_eq!(v["session_id"], sid);
    assert!(v["chat_id"].is_string());
}

#[tokio::test]
async fn plexus_send_reaches_browser() {
    let (_state, port) = spawn_gateway().await;
    let mut plexus = connect_plexus(port).await;
    let mut browser = connect_browser(port, "user-2").await;

    let sid = session_id_for("user-2").await;
    browser
        .send(WsMessage::Text(
            json!({"type":"message","session_id":sid,"content":"ping"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let v = recv_non_ping(&mut plexus).await;
    let chat_id = v["chat_id"].as_str().unwrap().to_string();

    plexus
        .send(WsMessage::Text(
            json!({
                "type": "send",
                "chat_id": chat_id,
                "session_id": sid,
                "content": "pong",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let v = recv_non_ping(&mut browser).await;
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"], "pong");
    assert_eq!(v["session_id"], sid);
}

#[tokio::test]
async fn progress_hint_forwarded() {
    let (_state, port) = spawn_gateway().await;
    let mut plexus = connect_plexus(port).await;
    let mut browser = connect_browser(port, "user-p").await;

    let sid = session_id_for("user-p").await;
    browser
        .send(WsMessage::Text(
            json!({"type":"message","session_id":sid,"content":"do it"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let v = recv_non_ping(&mut plexus).await;
    let chat_id = v["chat_id"].as_str().unwrap().to_string();

    plexus
        .send(WsMessage::Text(
            json!({
                "type": "send",
                "chat_id": chat_id,
                "session_id": sid,
                "content": "Executing shell on laptop...",
                "metadata": { "_progress": true },
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let v = recv_non_ping(&mut browser).await;
    assert_eq!(v["type"], "progress");
    assert_eq!(v["content"], "Executing shell on laptop...");
}

#[tokio::test]
async fn media_attachments_forwarded_both_directions() {
    let (_state, port) = spawn_gateway().await;
    let mut plexus = connect_plexus(port).await;
    let mut browser = connect_browser(port, "user-m").await;

    let sid = session_id_for("user-m").await;
    browser
        .send(WsMessage::Text(
            json!({
                "type":"message",
                "session_id":sid,
                "content":"check this",
                "media":["file1:pic.png"],
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let v = recv_non_ping(&mut plexus).await;
    assert_eq!(v["media"], json!(["file1:pic.png"]));
    let chat_id = v["chat_id"].as_str().unwrap().to_string();

    plexus
        .send(WsMessage::Text(
            json!({
                "type": "send",
                "chat_id": chat_id,
                "session_id": sid,
                "content": "here you go",
                "metadata": { "media": ["https://example.com/result.png"] },
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let v = recv_non_ping(&mut browser).await;
    assert_eq!(v["type"], "message");
    assert_eq!(v["media"], json!(["https://example.com/result.png"]));
}

#[tokio::test]
async fn browser_without_plexus_gets_error() {
    let (_state, port) = spawn_gateway().await;
    let mut browser = connect_browser(port, "user-3").await;
    let sid = session_id_for("user-3").await;

    browser
        .send(WsMessage::Text(
            json!({"type":"message","session_id":sid,"content":"lonely"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let v = recv_non_ping(&mut browser).await;
    assert_eq!(v["type"], "error");
    assert!(v["reason"].as_str().unwrap().contains("not connected"));
}

#[tokio::test]
async fn invalid_session_prefix_rejected() {
    let (_state, port) = spawn_gateway().await;
    let _plexus = connect_plexus(port).await;
    let mut browser = connect_browser(port, "user-x").await;

    // Malicious: claims session belonging to user-y.
    browser
        .send(WsMessage::Text(
            json!({
                "type":"message",
                "session_id":"gateway:user-y:abc",
                "content":"spoofing",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let v = recv_non_ping(&mut browser).await;
    assert_eq!(v["type"], "error");
    assert!(v["reason"].as_str().unwrap().contains("invalid session_id"));
    // Connection remains open — send another frame with a valid session.
}

#[tokio::test]
async fn invalid_jwt_rejected() {
    let (_state, port) = spawn_gateway().await;
    let res = connect_async(format!(
        "ws://127.0.0.1:{port}/ws/chat?token=not-a-valid-jwt"
    ))
    .await;
    assert!(res.is_err(), "expected upgrade rejection");
}

#[tokio::test]
async fn duplicate_plexus_rejected() {
    let (_state, port) = spawn_gateway().await;
    let _p1 = connect_plexus(port).await;
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

#[tokio::test]
async fn reader_writer_leak_free() {
    let (state, port) = spawn_gateway().await;
    // Spin up 50 browsers and close them.
    for i in 0..50 {
        let ws = connect_browser(port, &format!("user-{i}")).await;
        drop(ws);
    }
    // Wait briefly for cleanup.
    for _ in 0..40 {
        if state.browsers.len() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(state.browsers.len(), 0, "state.browsers should be empty after disconnects");
}

#[tokio::test]
async fn healthz_reports_plexus_connected() {
    let (_state, port) = spawn_gateway().await;
    let client = reqwest::Client::new();

    // Before plexus connects
    let resp: Value = client
        .get(format!("http://127.0.0.1:{port}/healthz"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["plexus_connected"], false);

    // After plexus connects
    let _plexus = connect_plexus(port).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let resp: Value = client
        .get(format!("http://127.0.0.1:{port}/healthz"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["plexus_connected"], true);
}

#[tokio::test]
async fn proxy_rejects_path_traversal() {
    let (_state, port) = spawn_gateway().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/api/../etc/passwd"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
}

#[tokio::test]
async fn proxy_rejects_oversized_request() {
    let (_state, port) = spawn_gateway().await;
    let client = reqwest::Client::new();
    let big = vec![0u8; 26 * 1024 * 1024]; // 26 MB
    let resp = client
        .post(format!("http://127.0.0.1:{port}/api/files"))
        .header("authorization", format!("Bearer {}", valid_jwt("u")))
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
}

#[tokio::test]
async fn proxy_rejects_oversized_response_via_content_length() {
    // Bring up a mock upstream that advertises a huge Content-Length.
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        use axum::routing::get;
        let app: axum::Router = axum::Router::new().route(
            "/api/big",
            get(|| async {
                let mut resp = axum::response::Response::new(axum::body::Body::from("x"));
                resp.headers_mut().insert(
                    axum::http::header::CONTENT_LENGTH,
                    axum::http::HeaderValue::from(30u64 * 1024 * 1024),
                );
                resp
            }),
        );
        axum::serve(upstream_listener, app).await.unwrap();
    });

    let (_state, port) = spawn_gateway_with(test_config(
        0,
        &format!("http://127.0.0.1:{upstream_port}"),
        "/tmp",
    ))
    .await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/api/big"))
        .header("authorization", format!("Bearer {}", valid_jwt("u")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "upstream_too_large");
}

#[tokio::test]
async fn origin_allowlist_rejects_disallowed() {
    let mut config = test_config(0, "http://127.0.0.1:1", "/tmp");
    config.allowed_origins = vec!["https://allowed.example.com".into()];
    let (_state, port) = spawn_gateway_with(config).await;

    // Try to connect with a mismatched Origin header.
    let jwt = valid_jwt("u");
    let mut req = format!("ws://127.0.0.1:{port}/ws/chat?token={jwt}")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "origin",
        "https://evil.example.com".parse().unwrap(),
    );
    let res = tokio_tungstenite::connect_async(req).await;
    assert!(res.is_err(), "expected origin rejection");
}

#[tokio::test]
async fn graceful_shutdown_closes_sockets() {
    let (state, port) = spawn_gateway().await;
    let _plexus = connect_plexus(port).await;
    let _browser = connect_browser(port, "user-s").await;

    state.shutdown.cancel();

    // Wait for serve() to exit — bind the port with a fresh listener.
    for _ in 0..100 {
        if tokio::net::TcpListener::bind(("127.0.0.1", port)).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("gateway did not shut down within 5s");
}
```

Add the missing import at the top of the test file: `use tokio_tungstenite::tungstenite::client::IntoClientRequest;`. (Required by `origin_allowlist_rejects_disallowed` for building a custom request.)

- [ ] **Step 2: Run tests**

Run: `cargo test --package plexus-gateway`
Expected: All unit tests (jwt, routing, config) + 14 integration tests pass.

- [ ] **Step 3: Commit**

```bash
git add plexus-gateway/tests/
git commit -m "test(gateway): comprehensive integration tests covering backpressure, progress, media, shutdown"
```

- [ ] **Step 4: USER VALIDATION GATE**

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
3. **Browser WebSocket:** Connect to `ws://localhost:9090/ws/chat?token=<JWT>`. Send `{"type":"message","session_id":"gateway:<user_id>:<uuid>","content":"hello"}` (replace `<user_id>` with the JWT `sub` and `<uuid>` with any UUID). Plexus-server should receive it and reply.
4. **Session prefix check:** Send a message with `session_id` that does NOT start with `gateway:<user_id>:` — expect `{"type":"error","reason":"invalid session_id"}`.
5. **Error flow:** Kill plexus-server. Send a browser message. Expect `{"type":"error","reason":"Plexus server not connected"}`.
6. **Health check:** `GET http://localhost:9090/healthz` → `{"status":"ok","plexus_connected":<bool>,"browsers":<int>}`.
7. **Static files:** Create a dummy `plexus-frontend/dist/index.html` and verify `GET http://localhost:9090/` returns it.

Wait for the user to confirm "Phase 1 validated" before beginning Task 12.

---

## Phase 2 — plexus-frontend

### Task 12: Frontend Project Scaffold

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

### Task 13: TypeScript Types + API Wrapper

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

// WebSocket message shapes (browser ↔ gateway) — PROTOCOL.md r2
export type WsIncoming =
  | { type: 'message'; session_id: string; content: string; media?: string[] }
  | { type: 'progress'; session_id: string; content: string }
  | { type: 'error'; reason: string }
  | { type: 'ping' }

export type WsOutgoing =
  | { type: 'message'; session_id: string; content: string; media?: string[] }
  | { type: 'pong' }
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

### Task 14: Auth Store + Login Page + Router Skeleton

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

### Task 15: WebSocket Manager

**Files:**
- Create: `plexus-frontend/src/lib/ws.ts`

- [ ] **Step 1: Create `ws.ts`**

```ts
// Singleton WebSocket manager with jittered auto-reconnect and
// terminal auth-failure handling. Replies to server pings with pongs.

import type { WsIncoming, WsOutgoing } from '@/lib/types'

type Status = 'connecting' | 'open' | 'closed' | 'auth_failed'
type Listener = (msg: WsIncoming) => void
type StatusListener = (status: Status) => void

const BASE_DELAYS_MS = [1_000, 2_000, 4_000, 8_000, 16_000, 30_000]
const MAX_DELAY_MS = 30_000

function jitter(delay: number): number {
  // 75%–125% of the base delay to spread reconnect stampedes.
  return Math.floor(delay * (0.75 + Math.random() * 0.5))
}

class WebSocketManager {
  private ws: WebSocket | null = null
  private listeners = new Set<Listener>()
  private statusListeners = new Set<StatusListener>()
  private token: string | null = null
  private reconnectAttempts = 0
  private reconnectTimer: number | null = null
  private shouldReconnect = false
  private status: Status = 'closed'

  connect(token: string) {
    // Idempotent — calling with the same token while already connected is a no-op.
    if (this.token === token && (this.status === 'open' || this.status === 'connecting')) {
      return
    }
    this.token = token
    this.shouldReconnect = true
    this.reconnectAttempts = 0
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
    this.token = null
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
    // Emit current status immediately so late subscribers know the state.
    l(this.status)
    return () => this.statusListeners.delete(l)
  }

  getStatus(): Status {
    return this.status
  }

  private dial() {
    if (!this.token) return
    this.setStatus('connecting')
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const url = `${protocol}//${window.location.host}/ws/chat?token=${encodeURIComponent(this.token)}`
    let ws: WebSocket
    try {
      ws = new WebSocket(url)
    } catch (err) {
      console.warn('ws: construct failed', err)
      this.scheduleReconnect()
      return
    }
    this.ws = ws

    ws.onopen = () => {
      this.reconnectAttempts = 0
      this.setStatus('open')
    }
    ws.onmessage = (e) => {
      let msg: WsIncoming
      try {
        msg = JSON.parse(e.data) as WsIncoming
      } catch (err) {
        console.warn('ws: invalid message', err)
        return
      }
      // Gateway ping → reply with pong (transparent to app listeners).
      if ((msg as any).type === 'ping') {
        this.ws?.send(JSON.stringify({ type: 'pong' }))
        return
      }
      // Terminal auth failure: stop reconnecting, emit status, bubble to listeners.
      if (msg.type === 'error' && (msg.reason === 'invalid token' || msg.reason === 'unauthorized')) {
        this.shouldReconnect = false
        this.setStatus('auth_failed')
        this.listeners.forEach((l) => l(msg))
        return
      }
      this.listeners.forEach((l) => l(msg))
    }
    ws.onclose = (ev) => {
      this.ws = null
      // 4401 = custom "unauthorized" close from future server hardening; currently we rely on
      // HTTP 401 at upgrade (which shows up as onerror + onclose without open).
      if (ev.code === 4401 || ev.code === 1008) {
        this.shouldReconnect = false
        this.setStatus('auth_failed')
        return
      }
      this.setStatus('closed')
      if (this.shouldReconnect) this.scheduleReconnect()
    }
    ws.onerror = (e) => {
      console.warn('ws: error', e)
    }
  }

  private scheduleReconnect() {
    if (this.reconnectTimer !== null) return
    const base = BASE_DELAYS_MS[Math.min(this.reconnectAttempts, BASE_DELAYS_MS.length - 1)]
    const delay = jitter(Math.min(base, MAX_DELAY_MS))
    this.reconnectAttempts++
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null
      this.dial()
    }, delay)
  }

  private setStatus(status: Status) {
    this.status = status
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
git commit -m "feat(frontend): WebSocket manager with jittered reconnect and auth-failure terminal state"
```

---

### Task 16: Chat Store

**Files:**
- Create: `plexus-frontend/src/store/chat.ts`

**Notes:**
- Browser owns session state. The store does NOT handle `session_created` / `session_switched` — those messages no longer exist in PROTOCOL.md r2.
- `setCurrentSession` updates state only; there is no WS message sent.
- `sendMessage` takes an explicit `sessionId` (from the URL) rather than relying on a mutable current-session field.
- `loadMessages` **merges** into existing state using a dedup set keyed on `message_id`.
- `handleIncomingMessage` appends unconditionally (it's live, not historical).
- `init()` is idempotent — the WS listener cleanups are stored in module-level variables so StrictMode double-invocations don't double-register.

- [ ] **Step 1: Create `chat.ts`**

```ts
import { create } from 'zustand'
import { api } from '@/lib/api'
import { wsManager } from '@/lib/ws'
import { useAuthStore } from '@/store/auth'
import type { DbMessage, Session, WsIncoming, WsOutgoing } from '@/lib/types'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  media?: string[]
  created_at: string
}

type WsStatus = 'connecting' | 'open' | 'closed' | 'auth_failed'

interface ChatState {
  sessions: Session[]
  currentSessionId: string | null
  messagesBySession: Record<string, ChatMessage[]>
  messageIdsBySession: Record<string, Set<string>>
  progressBySession: Record<string, string | null>
  wsStatus: WsStatus

  init: () => void
  loadSessions: () => Promise<void>
  loadMessages: (sessionId: string) => Promise<void>
  setCurrentSession: (sessionId: string | null) => void
  sendMessage: (sessionId: string, content: string, media?: string[]) => void

  // Internal dispatches from the WS manager
  handleIncomingMessage: (sessionId: string, content: string, media?: string[]) => void
  setProgressHint: (sessionId: string, hint: string) => void
  clearProgress: (sessionId: string) => void
  handleError: (reason: string) => void
}

// Module-scoped flags so `init()` is idempotent across React.StrictMode
// double-mounts and navigation effects.
let wsUnsubMessage: (() => void) | null = null
let wsUnsubStatus: (() => void) | null = null

export const useChatStore = create<ChatState>((set, get) => ({
  sessions: [],
  currentSessionId: null,
  messagesBySession: {},
  messageIdsBySession: {},
  progressBySession: {},
  wsStatus: 'closed',

  init() {
    if (wsUnsubMessage || wsUnsubStatus) return
    wsUnsubMessage = wsManager.onMessage((msg) => dispatchIncoming(get, msg))
    wsUnsubStatus = wsManager.onStatus((status) => set({ wsStatus: status }))
  },

  async loadSessions() {
    const sessions = await api<Session[]>('/api/sessions')
    set({ sessions })
  },

  async loadMessages(sessionId) {
    const rows = await api<DbMessage[]>(
      `/api/sessions/${encodeURIComponent(sessionId)}/messages?limit=200`,
    )
    const snapshot: ChatMessage[] = rows
      .filter((r) => r.role === 'user' || r.role === 'assistant')
      .map((r) => ({
        id: r.message_id,
        role: r.role as ChatMessage['role'],
        content: r.content,
        created_at: r.created_at,
      }))

    set((s) => {
      const existing = s.messagesBySession[sessionId] ?? []
      const existingIds = s.messageIdsBySession[sessionId] ?? new Set<string>()
      // Merge: keep all existing messages (including WS-delivered ones that
      // arrived during the REST fetch), append snapshot entries that are not
      // already present.
      const merged = [...existing]
      const mergedIds = new Set(existingIds)
      for (const m of snapshot) {
        if (!mergedIds.has(m.id)) {
          merged.push(m)
          mergedIds.add(m.id)
        }
      }
      // Sort by created_at so merged order is stable.
      merged.sort((a, b) => a.created_at.localeCompare(b.created_at))
      return {
        messagesBySession: { ...s.messagesBySession, [sessionId]: merged },
        messageIdsBySession: { ...s.messageIdsBySession, [sessionId]: mergedIds },
      }
    })
  },

  setCurrentSession(sessionId) {
    set((s) => {
      const progress = { ...s.progressBySession }
      if (s.currentSessionId && s.currentSessionId !== sessionId) {
        // Clear the previous session's progress hint — we don't know if the
        // agent is still running and the hint is per-view anyway.
        progress[s.currentSessionId] = null
      }
      return { currentSessionId: sessionId, progressBySession: progress }
    })
  },

  sendMessage(sessionId, content, media) {
    // Optimistic local echo — random id so REST merge won't dedup.
    const local: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content,
      media,
      created_at: new Date().toISOString(),
    }
    set((s) => {
      const existing = s.messagesBySession[sessionId] ?? []
      const ids = new Set(s.messageIdsBySession[sessionId] ?? [])
      ids.add(local.id)
      return {
        messagesBySession: {
          ...s.messagesBySession,
          [sessionId]: [...existing, local],
        },
        messageIdsBySession: { ...s.messageIdsBySession, [sessionId]: ids },
      }
    })
    const msg: WsOutgoing = { type: 'message', session_id: sessionId, content, media }
    wsManager.send(msg)
  },

  handleIncomingMessage(sessionId, content, media) {
    const entry: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'assistant',
      content,
      media,
      created_at: new Date().toISOString(),
    }
    set((s) => {
      const existing = s.messagesBySession[sessionId] ?? []
      const ids = new Set(s.messageIdsBySession[sessionId] ?? [])
      ids.add(entry.id)
      return {
        messagesBySession: {
          ...s.messagesBySession,
          [sessionId]: [...existing, entry],
        },
        messageIdsBySession: { ...s.messageIdsBySession, [sessionId]: ids },
        progressBySession: { ...s.progressBySession, [sessionId]: null },
      }
    })
  },

  setProgressHint(sessionId, hint) {
    set((s) => ({
      progressBySession: { ...s.progressBySession, [sessionId]: hint },
    }))
  },

  clearProgress(sessionId) {
    set((s) => ({
      progressBySession: { ...s.progressBySession, [sessionId]: null },
    }))
  },

  handleError(reason) {
    console.warn('ws error:', reason)
    if (reason === 'invalid token' || reason === 'unauthorized') {
      useAuthStore.getState().logout()
    }
  },
}))

function dispatchIncoming(
  get: () => ChatState,
  msg: WsIncoming,
) {
  switch (msg.type) {
    case 'message':
      get().handleIncomingMessage(msg.session_id, msg.content, msg.media)
      break
    case 'progress':
      get().setProgressHint(msg.session_id, msg.content)
      break
    case 'error':
      get().handleError(msg.reason)
      break
  }
}
```

The `WsIncoming` / `WsOutgoing` types in `lib/types.ts` should already match PROTOCOL.md r2 (no `session_created`, `session_switched`, `new_session`, or `switch_session` variants). Verify by `grep`ing — if any old type names remain, update them before proceeding.

- [ ] **Step 2: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add plexus-frontend/src/store/chat.ts
git commit -m "feat(frontend): chat store with REST/WS merge and idempotent init"
```

---

### Task 17: Devices Store + Markdown Component

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

### Task 18: Chat Page Components (Sidebar, Message, Input, ProgressHint, DeviceStatusBar)

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
import { NavLink, useNavigate } from 'react-router-dom'
import { ChevronLeft, ChevronRight, Plus, Settings, ShieldCheck, LogOut } from 'lucide-react'
import { useChatStore } from '@/store/chat'
import { useAuthStore } from '@/store/auth'

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false)
  const sessions = useChatStore((s) => s.sessions)
  const currentSessionId = useChatStore((s) => s.currentSessionId)
  const isAdmin = useAuthStore((s) => s.isAdmin)
  const userId = useAuthStore((s) => s.userId)
  const logout = useAuthStore((s) => s.logout)
  const navigate = useNavigate()

  function openNewChat() {
    const newId = `gateway:${userId}:${crypto.randomUUID()}`
    navigate(`/chat/${encodeURIComponent(newId)}`)
  }

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
        onClick={openNewChat}
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
              onClick={() => navigate(`/chat/${encodeURIComponent(s.session_id)}`)}
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

### Task 19: Wire Up the Chat Page

**Files:**
- Replace: `plexus-frontend/src/pages/Chat.tsx`

- [ ] **Step 1: Implement Chat page with URL-driven session routing**

```tsx
import { useEffect } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { useAuthStore } from '@/store/auth'
import { useChatStore } from '@/store/chat'
import { wsManager } from '@/lib/ws'
import Sidebar from '@/components/Sidebar'
import MessageList from '@/components/MessageList'
import ChatInput from '@/components/ChatInput'
import DeviceStatusBar from '@/components/DeviceStatusBar'

export default function Chat() {
  const token = useAuthStore((s) => s.token)
  const userId = useAuthStore((s) => s.userId)
  const init = useChatStore((s) => s.init)
  const loadSessions = useChatStore((s) => s.loadSessions)
  const loadMessages = useChatStore((s) => s.loadMessages)
  const sendMessage = useChatStore((s) => s.sendMessage)
  const setCurrentSession = useChatStore((s) => s.setCurrentSession)
  const messagesBySession = useChatStore((s) => s.messagesBySession)
  const progressBySession = useChatStore((s) => s.progressBySession)

  const { sessionId: paramSessionId } = useParams<{ sessionId: string }>()
  const navigate = useNavigate()

  // Step 1: if URL has no session, generate one and redirect.
  useEffect(() => {
    if (!userId) return
    if (!paramSessionId) {
      const newId = `gateway:${userId}:${crypto.randomUUID()}`
      navigate(`/chat/${encodeURIComponent(newId)}`, { replace: true })
    }
  }, [paramSessionId, userId, navigate])

  // Step 2: idempotent init of store listeners (guarded by flags in the store).
  // Open WebSocket (idempotent).
  useEffect(() => {
    init()
    if (token) wsManager.connect(token)
    loadSessions().catch(console.error)
    // Do NOT disconnect on unmount — unmount can happen during navigation
    // between /chat and /settings. Disconnect only on explicit logout.
  }, [token, init, loadSessions])

  // Step 3: sync currentSessionId with URL; load history on change.
  useEffect(() => {
    if (!paramSessionId) return
    setCurrentSession(paramSessionId)
    if (!messagesBySession[paramSessionId]) {
      loadMessages(paramSessionId).catch((err) => {
        // 404 is expected for brand-new sessions — not an error.
        if (err?.status !== 404) console.error(err)
      })
    }
  }, [paramSessionId, setCurrentSession, loadMessages, messagesBySession])

  const currentSessionId = paramSessionId ?? null
  const messages = currentSessionId ? messagesBySession[currentSessionId] ?? [] : []
  const progress = currentSessionId ? progressBySession[currentSessionId] ?? null : null
  const empty = messages.length === 0

  function onSendText(text: string) {
    if (!currentSessionId) return
    sendMessage(currentSessionId, text)
  }

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
              <ChatInput onSend={onSendText} />
            </div>
          </div>
        ) : (
          <>
            <MessageList messages={messages} progressHint={progress} />
            <div className="border-t border-plex-border px-6 py-3">
              <ChatInput onSend={onSendText} />
            </div>
          </>
        )}
      </main>
    </div>
  )
}
```

The `App.tsx` router already defines `/chat/:sessionId` from Task 14 — no change needed there.

- [ ] **Step 2: Build**

Run: `cd plexus-frontend && npm run build`
Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add plexus-frontend/src/pages/Chat.tsx
git commit -m "feat(frontend): URL-driven session routing in chat page"
```

---

### Task 20: Settings Page — Profile Tab

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

### Task 21: Settings — Devices Tab

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

### Task 22: Settings — Channels, Skills, Cron Tabs

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

### Task 23: Admin Page

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

### Task 24: Component Smoke Tests + Chat Store Tests

**Files:**
- Create: `plexus-frontend/src/components/__tests__/Message.test.tsx`
- Create: `plexus-frontend/src/components/__tests__/ChatInput.test.tsx`
- Create: `plexus-frontend/src/components/__tests__/ProgressHint.test.tsx`
- Create: `plexus-frontend/src/store/__tests__/chat.test.ts`

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

- [ ] **Step 4: Test chat store REST/WS merge, progress lifecycle, idempotent init**

Create `plexus-frontend/src/store/__tests__/chat.test.ts`:

```ts
import { beforeEach, describe, expect, test, vi } from 'vitest'

// Mock the fetch wrapper used by the store so we can hand-craft REST responses.
vi.mock('@/lib/api', () => ({
  api: vi.fn(),
}))
vi.mock('@/lib/ws', () => ({
  wsManager: {
    onMessage: vi.fn(() => () => {}),
    onStatus: vi.fn(() => () => {}),
    send: vi.fn(),
  },
}))

import { api } from '@/lib/api'
import { wsManager } from '@/lib/ws'
import { useChatStore } from '@/store/chat'

describe('chat store', () => {
  beforeEach(() => {
    useChatStore.setState({
      sessions: [],
      currentSessionId: null,
      messagesBySession: {},
      messageIdsBySession: {},
      progressBySession: {},
      wsStatus: 'closed',
    })
    vi.clearAllMocks()
  })

  test('init is idempotent across multiple calls', () => {
    useChatStore.getState().init()
    useChatStore.getState().init()
    useChatStore.getState().init()
    expect((wsManager.onMessage as any).mock.calls.length).toBe(1)
    expect((wsManager.onStatus as any).mock.calls.length).toBe(1)
  })

  test('loadMessages merges with existing WS-delivered entries', async () => {
    const sid = 'gateway:u:abc'
    // First, simulate a WS message arriving before REST completes.
    useChatStore.getState().handleIncomingMessage(sid, 'live reply')
    expect(useChatStore.getState().messagesBySession[sid]).toHaveLength(1)

    // Now REST history returns ONE row with a distinct message_id.
    ;(api as any).mockResolvedValue([
      {
        message_id: 'server-1',
        role: 'assistant',
        content: 'historical reply',
        tool_call_id: null,
        tool_name: null,
        tool_arguments: null,
        created_at: '2026-04-10T00:00:00Z',
      },
    ])
    await useChatStore.getState().loadMessages(sid)

    // The WS entry must still be present after merge.
    const after = useChatStore.getState().messagesBySession[sid]
    expect(after.length).toBe(2)
    expect(after.some((m) => m.content === 'live reply')).toBe(true)
    expect(after.some((m) => m.content === 'historical reply')).toBe(true)
  })

  test('loadMessages deduplicates repeated REST fetches by message_id', async () => {
    const sid = 'gateway:u:abc'
    const row = {
      message_id: 'srv-42',
      role: 'assistant',
      content: 'hi',
      tool_call_id: null,
      tool_name: null,
      tool_arguments: null,
      created_at: '2026-04-10T00:00:00Z',
    }
    ;(api as any).mockResolvedValue([row])
    await useChatStore.getState().loadMessages(sid)
    await useChatStore.getState().loadMessages(sid)
    expect(useChatStore.getState().messagesBySession[sid]).toHaveLength(1)
  })

  test('progress hint cleared on final message', () => {
    const sid = 'gateway:u:abc'
    useChatStore.getState().setProgressHint(sid, 'running tool...')
    expect(useChatStore.getState().progressBySession[sid]).toBe('running tool...')
    useChatStore.getState().handleIncomingMessage(sid, 'done')
    expect(useChatStore.getState().progressBySession[sid]).toBeNull()
  })

  test('progress hint cleared on session switch', () => {
    const a = 'gateway:u:aaa'
    const b = 'gateway:u:bbb'
    useChatStore.setState({ currentSessionId: a })
    useChatStore.getState().setProgressHint(a, 'doing')
    useChatStore.getState().setCurrentSession(b)
    expect(useChatStore.getState().progressBySession[a]).toBeNull()
  })
})
```

- [ ] **Step 5: Run tests**

Run: `cd plexus-frontend && npm test`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add plexus-frontend/src/components/__tests__/ plexus-frontend/src/store/__tests__/
git commit -m "test(frontend): component + chat store tests"
```

---

### Task 25: End-to-End Manual Validation + Workspace Integration Check

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

**Spec coverage (r2):**

- Gateway crate layout → Task 1
- Config + allowed_origins → Task 2
- JWT validation → Task 3
- State (DashMap + plexus sender + shutdown token) → Task 4
- Static files + SPA fallback → Task 5
- Non-blocking routing with eviction → Task 6
- `/ws/chat` browser handler (prefix check, keepalive, lifecycle) → Task 7
- `/ws/plexus` server handler (keepalive, graceful shutdown) → Task 8
- REST proxy + `/healthz` + strict CORS → Task 9
- Library split + graceful shutdown → Task 10
- Integration tests + Postman gate → Task 11
- Frontend scaffold → Task 12
- Types + API wrapper → Task 13
- Auth store + login + router → Task 14
- WebSocket manager (jitter, auth_failed) → Task 15
- Chat store (REST/WS merge, idempotent init, no session_created) → Task 16
- Devices store + markdown → Task 17
- Chat components → Task 18
- Chat page (URL-driven session) → Task 19
- Settings profile → Task 20
- Settings devices → Task 21
- Settings channels/skills/cron → Task 22
- Admin page → Task 23
- Component + chat store tests → Task 24
- Full-stack validation → Task 25

**Blocking-issue coverage (Codex r1 review):**

1. Session ID mismatch → fixed: browser owns session, gateway validates prefix, server already has session_id in OutboundEvent and now emits it in the gateway channel (1-line server change, already committed).
2. Writer task lifecycle leak → fixed in Tasks 7 and 8: explicit `drop(outbound_tx)` before `writer.await`; `state.plexus.write().await.take()` before `writer.await` in plexus handler.
3. Head-of-line blocking in routing → fixed in Task 6: `route_send` is non-blocking; uses `try_send`; clones handles out of DashMap shards before any await; evicts slow browsers on final-message overflow.
4. Proxy response size not enforced → fixed in Task 9: Content-Length fast path + running counter over `bytes_stream()`.
5. Frontend session boot race → fixed via new session model: browser owns session state, URL is canonical, chat store merges REST/WS with dedup by message_id.

**Strong-rec coverage:**

1. CORS + Origin check → Tasks 2 (Config), 7 (WS chat Origin check), 9 (tightened CorsLayer).
2. WS jitter + auth failure → Task 15.
3. Chat store idempotent init → Task 16 + Task 24 test.
4. `/healthz` + graceful shutdown → Tasks 9 and 10.
5. Ping/pong keepalive → Task 7 (browser side) and Task 8 (plexus side).
6. Expanded test matrix → Task 11 (integration) and Task 24 (chat store tests).

**Placeholder check:** No TBDs, no "add appropriate error handling" handwaves, no "similar to Task N" references without code.

**Type consistency:** `BrowserConnection`, `OutboundFrame` (with `Close` variant), `AppState`, `Claims`, `WsIncoming` (r2), `WsOutgoing` (r2), `ChatMessage` all match across tasks. Zustand stores use consistent selector patterns. Store's `setCurrentSession`/`sendMessage` signatures match the URL-driven Chat page consumers.

**Scope check:** Plan is gated between Phase 1 (Task 11) and Phase 2 (Task 12+) with explicit user validation. Both phases ship complete, testable software.
