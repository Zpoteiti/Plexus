# M3 — Gateway + Frontend Design

**Status:** r4 — simplified after implementation brainstorm, ready for plan write
**Date:** 2026-04-10
**Scope:** Build `plexus-gateway` (Rust) and `plexus-frontend` (React) from scratch

## Revision History

**2026-04-10 (r4):** Simplified after implementation brainstorm. Architecture unchanged; r4 removes over-engineering that doesn't pay off at M3 scale.
- **Single outbound channel:** merged `data_tx` / `ctrl_tx` into one `mpsc::channel(64)`. `OutboundFrame` enum now includes `Ping` alongside `Message`, `Progress`, `Error`. At M3 scale (≤1,000 browsers), a backed-up queue cannot realistically starve pings because keepalive fires every 30s and the channel drains sub-millisecond per frame. If a channel is genuinely full for 2+ minutes, the browser is dead anyway and gets evicted.
- **Simple missed-pong counter:** replaced `AtomicI64` timestamp tracking with `AtomicU32` counter. Ping increments it, pong resets it. If counter > 3 (~2 min of silence), cancel the connection. No timestamp math.
- **No sender_id fallback scan:** routing uses direct `chat_id` lookup only. If no match, warn and drop. Cron job outputs are still saved to DB by plexus-server regardless of gateway delivery; users see them via `GET /api/sessions/{id}/messages`. Fallback scan is M4.
- **Simple response body limit:** `reqwest .bytes()` with a length check rather than streaming byte counter. Sufficient for M3 payload sizes.
- **Public REST prefix widened:** JWT validation skipped for all `/api/auth/*` paths (not just `/api/auth/login` and `/api/auth/register`), so future auth endpoints like password reset work without gateway changes.

**2026-04-10 (r3):** Tightened after second Codex review. The architecture is unchanged; r3 closes implementation-detail gaps identified during r2 plan review.
- **Per-connection `CancellationToken`:** each browser gets a child token of `state.shutdown`. The reader, writer, and keepalive tasks all `select!` on it. Shutdown, eviction, keepalive timeout, and graceful signals all trigger the same exit path without ordering pitfalls.
- **Separate control channel for keepalive:** replaced in r4 with single channel.
- **Plexus-side keepalive removed:** TCP keepalive + plexus-server reconnect loop are sufficient for M3. PROTOCOL.md r3 no longer mentions `/ws/plexus` app-level ping/pong. Revisit if ops experience shows it's needed.
- **Progress overflow wording:** aligned to "drop on full" (tokio mpsc `try_send` drops the newest). No more "drop oldest" claim.
- **Frontend merge semantics:** claim narrowed from "dedup" to "initial-load merge with no clobber." We do not claim REST/WS can both render the same assistant message only once, because WS final messages currently get client-assigned IDs.
- **Frontend `auth_failed` state removed:** gateway-side HTTP 401 on upgrade manifests as `onerror` → `onclose` without `onopen`. Detecting this reliably would require heuristics. Instead, rely on REST 401 → logout and on the user noticing a stalled connection. Simpler, honest.
- **Deployment docs updated:** DEPLOYMENT.md r3 documents `PLEXUS_ALLOWED_ORIGINS`, strict origin requirement for prod, reverse-proxy `Origin` preservation, and JWT query-param redaction in access logs.

**2026-04-10 (r2):** Addressed first Codex review. Major changes:
- **Session model:** the browser now owns session state. Gateway is fully stateless w.r.t. sessions; `new_session` / `switch_session` / `session_created` / `session_switched` messages removed from the protocol. Every inbound `message` carries a `session_id`. Gateway validates the prefix against the JWT `sub`. See PROTOCOL.md r2.
- **Routing:** `/ws/plexus` reader loop is now strictly non-blocking. Slow browsers are evicted on final-message overflow instead of stalling the shared read loop. DashMap handles are cloned out of shards before any await.
- **Proxy response size limit:** enforced via streaming with a running byte counter and a `Content-Length` fast-path.
- **Graceful shutdown:** SIGTERM/SIGINT → stop accepting → close browsers → drain → exit.
- **Health check:** `GET /healthz` endpoint for load balancer readiness probes.
- **Test matrix:** expanded to cover backpressure, progress hints, media attachments, reconnect, proxy size limit, path traversal.

## Summary

M3 delivers two new crates/projects that complete the user-facing surface of Plexus:

1. **plexus-gateway** — a lightweight Rust binary that serves as a WebSocket hub, REST proxy, and static file server. It is a "pipe with auth": browsers and plexus-server both dial in as WebSocket clients, and the gateway routes messages between them by `chat_id`.
2. **plexus-frontend** — a React 19 + TypeScript SPA with three pages (Chat, Settings, Admin) styled in the **Cyberpunk Refined** visual direction (GitHub-dark base, neon green `#39ff14` accents, chat bubbles, rounded corners).

Both reference docs (`plexus-gateway/docs/*`, `plexus-frontend/docs/*`) are already written and frozen; this spec is the implementation plan that honors them.

## Non-Goals

- No new plexus-server features. The server already exposes all the APIs this scope needs, and tool progress hints already flow through `OutboundEvent { is_progress: true }`.
- No changes to the WebSocket protocol described in `plexus-gateway/docs/PROTOCOL.md`. The wire format is frozen.
- No Playwright or heavy end-to-end test suite for the frontend. M3 tests are type-check plus component smoke tests; validation is manual with Postman (gateway) and browser (frontend).

## Delivery Phases

M3 ships in two sequential phases with a validation gate between them:

1. **Phase 1 — Gateway.** Full build, unit + integration tests, user validates the gateway WS endpoints and REST proxy with Postman. No frontend work begins until the gateway is green.
2. **Phase 2 — Frontend.** Built against the already-validated gateway, end-to-end validation is "click around in the browser."

## Topology

Everyone is a WebSocket *client*. The gateway only accepts inbound connections — it never reaches out on the WebSocket layer.

```
Browser       --[ws client]--> gateway:/ws/chat     (JWT in query param)
plexus-server --[ws client]--> gateway:/ws/plexus   (shared token, first-message auth)
Browser       --[REST]-------> gateway:/api/*       → reverse-proxied to plexus-server
Browser       --[HTTP]-------> gateway:/            → serves plexus-frontend/dist/
```

If plexus-server is not connected when a browser sends a message, the gateway returns `{"type":"error","reason":"Plexus server not connected"}`. Browser connections stay alive. This matches the "different failure domains" rationale in `plexus-gateway/docs/DECISIONS.md`.

**Single-binary deployment.** The gateway serves the frontend `dist/` directory as a static fallback route. Production deployment is: build frontend → point `PLEXUS_FRONTEND_DIR` at `dist/` → run the gateway binary. One binary, one port, one URL.

---

## Phase 1 — plexus-gateway

### Crate Layout

New workspace member `plexus-gateway` added to `Cargo.toml`. Module layout mirrors `plexus-server`'s flat style, with a `ws/` subfolder for the two distinct WebSocket protocols (analogous to `plexus-server/src/channels/` grouping channel adapters):

```
plexus-gateway/
├── Cargo.toml
├── .env.example
└── src/
    ├── main.rs           — bootstrap, router, axum serve
    ├── config.rs         — env loading (dotenvy), Config struct
    ├── state.rs          — AppState (DashMap<chat_id, BrowserConnection>, Arc<RwLock<Option<PlexusSink>>>)
    ├── jwt.rs            — JWT validation, Claims struct
    ├── proxy.rs          — /api/* REST passthrough
    ├── static_files.rs   — frontend serving with SPA fallback
    ├── routing.rs        — chat_id → browser lookup, try_send dispatch
    └── ws/
        ├── mod.rs        — shared WS types (BrowserConnection, PlexusConnection)
        ├── chat.rs       — /ws/chat (browser WS handler)
        └── plexus.rs     — /ws/plexus (server WS handler, constant-time token)
```

### Dependencies

```toml
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
plexus-common = { path = "../plexus-common" }

[dev-dependencies]
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }  # only for integration test clients
```

`axum`'s built-in `ws` feature handles server-side WebSocket upgrades. `tokio-tungstenite` is only pulled in for integration tests that need a client to dial into the gateway. `plexus-common` is reused for error types (`ApiError`, `ErrorCode`) — no separate `error.rs` in the gateway.

### State

The gateway is deliberately **stateless w.r.t. sessions** — it only holds the live WebSocket connection table.

Each browser connection has a single bounded mpsc channel and a per-connection `CancellationToken`. A dedicated writer task owns the WebSocket sink and `select!`s over the channel plus the cancel token. Reader, writer, and keepalive tasks all `select!` on the same cancel token, so any exit condition (disconnect, eviction, keepalive timeout, graceful shutdown) propagates cleanly to all three.

```rust
pub struct AppState {
    pub config: Config,
    pub browsers: Arc<DashMap<String, BrowserConnection>>,         // chat_id → per-connection handle
    pub plexus:   Arc<RwLock<Option<tokio::sync::mpsc::Sender<serde_json::Value>>>>,  // single server sender
    pub http_client: reqwest::Client,                              // pooled, shared across all proxy requests
    pub shutdown: tokio_util::sync::CancellationToken,             // triggered on SIGTERM/SIGINT
}

#[derive(Clone)]
pub struct BrowserConnection {
    pub tx: tokio::sync::mpsc::Sender<OutboundFrame>,        // bounded(64): all outbound frames
    pub user_id: String,                                     // from JWT
    pub cancel:  tokio_util::sync::CancellationToken,        // child of state.shutdown; triggers full teardown
}

pub enum OutboundFrame {
    Message(serde_json::Value),  // final message — on queue-full, evict the browser
    Progress(serde_json::Value), // ephemeral — on queue-full, drop silently (tokio mpsc drops newest)
    Error(serde_json::Value),    // in-band error reply — sent by reader loop, never evicts
    Ping,                        // sent by keepalive task; writer emits `{"type":"ping"}`
}
```

**`BrowserConnection` is `Clone`** so the routing layer can clone it out of the DashMap shard before any async operation, never holding a shard guard across an await.

**No session state at the gateway.** The browser supplies `session_id` in every `message` frame. The gateway validates the prefix (`gateway:{user_id}:` where `user_id` matches the JWT `sub`) and forwards it to plexus-server. Plexus-server creates the DB row on first use (same pattern as Discord and Telegram channels). This makes gateway restarts trivially safe — no session state is lost because the gateway never owned it.

**Task lifecycle rules (prevents leaks):**

1. On connect, the reader creates `(tx, rx)` and `conn_cancel = state.shutdown.child_token()`. It inserts `BrowserConnection { tx: tx.clone(), user_id, cancel: conn_cancel.clone() }` into `state.browsers`.
2. Spawn the writer task with `(ws_sink, rx, conn_cancel.clone())`. Writer does `select!` over cancel and rx.
3. Spawn the keepalive task with `(tx.clone(), missed_pongs: Arc<AtomicU32>, conn_cancel.clone())`. It sends `OutboundFrame::Ping` every 30s via `tx.try_send(...)`. Increments `missed_pongs` on each ping. If `missed_pongs > 3` (~2 min of silence) or `try_send` fails (channel full), calls `conn_cancel.cancel()`. On cancel, the keepalive exits its loop.
4. Reader loop uses `select!` over `conn_cancel.cancelled()` and `ws_stream.next()`. On incoming `pong`, resets `missed_pongs.store(0, Relaxed)`. On cancel or stream end, fall through to cleanup.
5. Cleanup order on disconnect:
   a. `state.browsers.remove(chat_id)` drops the stored `tx` clone.
   b. `conn_cancel.cancel()` — signals writer and keepalive.
   c. `drop(tx)` on the reader's local clone.
   d. `writer.await` — writer exits via cancel arm of `select!` (it does not need all senders dropped, because the cancel token wins).
   e. Keepalive will exit on its own via cancel; no need to await it.
6. Routing (see "Routing" below) may hold short-lived handle clones. It only uses `try_send`, never awaits, and does not hold the DashMap shard guard across any operation.

**Writer select contract:**

```rust
// Writer task pseudocode
loop {
    tokio::select! {
        biased;
        _ = conn_cancel.cancelled() => break,
        Some(frame) = rx.recv() => {
            let text = match frame {
                OutboundFrame::Ping => "{\"type\":\"ping\"}".to_string(),
                OutboundFrame::Message(v) | OutboundFrame::Progress(v) | OutboundFrame::Error(v) => v.to_string(),
            };
            sink.send(Text(text)).await?;
        }
        else => break, // recv returned None
    }
}
// On exit: send a close frame, best effort.
let _ = sink.send(Message::Close(...)).await;
```

Integration tests verify leak-freedom by connecting 200 browsers and disconnecting them, then asserting `state.browsers.len() == 0` within 100 ms.

### Environment Variables

| Variable | Required | Default | Source |
|---|---|---|---|
| `PLEXUS_GATEWAY_TOKEN` | yes | — | shared secret for plexus-server auth |
| `JWT_SECRET` | yes | — | HMAC secret, must match server |
| `GATEWAY_PORT` | yes | — | listen port |
| `PLEXUS_SERVER_API_URL` | yes | — | upstream base URL for REST proxy |
| `PLEXUS_FRONTEND_DIR` | no | `./plexus-frontend/dist` | static file root |
| `PLEXUS_ALLOWED_ORIGINS` | no | `*` | comma-separated CORS/WS-Origin allow-list; `*` is dev-only |

`dotenvy` loads a `.env` file in the working directory at startup.

Production deployments **must** set `PLEXUS_ALLOWED_ORIGINS` to an explicit list. The default `*` exists only so `cargo run --package plexus-gateway` works locally without extra setup. The integration tests set it explicitly.

### WebSocket: `/ws/chat` (browsers)

Handler lives in `ws/chat.rs`. Flow:

1. **Origin check.** Read the `Origin` header. If `Config::origin_allowed(Some(origin))` returns false, return HTTP 403 before upgrade.
2. **JWT validation.** Extract the `token` query parameter (browsers cannot set `Authorization` on WS upgrade). Validate via `jwt::validate()`. On failure → HTTP 401 before upgrade; no resources allocated.
3. **Upgrade and set up per-connection state.** Allocate `chat_id = Uuid::new_v4()`. Create `(tx, rx)` with buffer 64. Create `conn_cancel = state.shutdown.child_token()`. Insert a `BrowserConnection` clone into `state.browsers[chat_id]`.
4. **Spawn writer task.** Owns the WebSocket sink, drains rx via `select!`, and watches `conn_cancel`. On cancel or channel closed, sends a WebSocket close frame and exits.
5. **Spawn keepalive task.** Sends `OutboundFrame::Ping` every 30 seconds via `tx.try_send(...)`. Tracks `missed_pongs: Arc<AtomicU32>` (reset by reader on incoming `pong`). Increments the counter on each ping. If `missed_pongs > 3` (~2 min of silence) or `tx.try_send(Ping)` fails (channel full), immediately calls `conn_cancel.cancel()`.
6. **Reader loop.** `select!` over `conn_cancel.cancelled()` and `ws_stream.next()`. Dispatch incoming JSON by `type`:
   - `message` → validate `session_id` starts with `gateway:{user_id}:` (strict string prefix check). On mismatch, enqueue `OutboundFrame::Error(json!({"type":"error","reason":"invalid session_id"}))` via `tx.try_send` and continue. On match, call `routing::forward_to_plexus(&state, &chat_id, &user_id, &parsed, &tx)`. If plexus is not connected, the forward routine enqueues an `Error` frame instead.
   - `pong` → `missed_pongs.store(0, Relaxed)`; no reply.
   - Unknown type → log warn, ignore.
7. **Cleanup on disconnect / cancel / shutdown signal.** Order:
   1. `state.browsers.remove(&chat_id)` — drops stored sender clone.
   2. `conn_cancel.cancel()` — signals writer and keepalive (idempotent if already cancelled).
   3. `drop(tx)` on the reader's local clone.
   4. `writer.await` — writer wakes via cancel arm immediately and exits.
   5. Keepalive exits via cancel on its own; no `await` or `abort` needed.

**No session bookkeeping.** The gateway does not remember "current session per chat_id". There are no `new_session` / `switch_session` / `session_created` / `session_switched` messages. The browser is responsible for its own session state.

**Per-connection backpressure.**
- **Progress frames:** routing does `conn.tx.try_send(OutboundFrame::Progress(...))`. On `Full`, the frame is silently dropped. Tokio mpsc's `try_send` drops the newest on full (not the oldest). This is acceptable because progress is ephemeral and the user already has a recent hint on screen.
- **Final frames:** routing does `conn.tx.try_send(OutboundFrame::Message(...))`. On `Full`, the browser is evicted: `state.browsers.remove(chat_id)` and `conn.cancel.cancel()`. The writer and keepalive tasks exit via cancel. This prevents one slow consumer from head-of-line blocking the `/ws/plexus` reader loop.
- **Error frames (reader-originated):** the reader uses `tx.try_send` for in-band error replies. On `Full`, the error is dropped and we rely on the eviction path to clean up. The error-reply drop is acceptable because a client in that state is already unhealthy.

### WebSocket: `/ws/plexus` (plexus-server, exactly one)

Handler lives in `ws/plexus.rs`. No application-level keepalive in M3 — TCP keepalive and plexus-server's own reconnect loop are relied on to detect dead connections.

Flow:

1. Upgrade immediately — no query auth.
2. Wait for the first text frame with a 5-second timeout.
3. Parse as `{"type":"auth","token": "..."}`. Any other shape → `auth_fail` + drop.
4. Compare `token` to `PLEXUS_GATEWAY_TOKEN` using `subtle::ConstantTimeEq::ct_eq()`. Length mismatch short-circuits to false (length is not a secret in practice).
5. Acquire `state.plexus.write().await`. If already `Some` → `auth_fail(reason="duplicate connection")` + drop. Otherwise create `(plexus_tx, plexus_rx)` with buffer 256, store `plexus_tx` in `state.plexus`. Create `plexus_cancel = state.shutdown.child_token()`.
6. Send `{"type":"auth_ok"}`.
7. Spawn the writer task with `(ws_sink, plexus_rx, plexus_cancel.clone())`. Writer does `select!` over cancel and recv; exits via cancel or closed channel.
8. Reader loop: `select!` over `plexus_cancel.cancelled()` and `ws_stream.next()`. For each `send` message, call `routing::route_send(&state, &parsed)` — this is a **synchronous `fn`** (not `async`) that uses `try_send` exclusively. Unknown `type` → warn and ignore.
9. **On disconnect, cancel, or shutdown signal.** Order:
   1. `state.plexus.write().await.take()` — drops the stored `plexus_tx` clone.
   2. `plexus_cancel.cancel()` — signals writer (idempotent if already cancelled).
   3. `drop(plexus_tx)` on any local clone (none in the current design, but documented for future additions).
   4. `writer.await` — writer wakes via cancel arm and exits.
10. Browsers continue serving normally. Their next `message` gets `{"type":"error","reason":"Plexus server not connected"}`.

### Routing

Handler lives in `routing.rs`. The contract is: **`route_send` is a synchronous `fn` with no `await`s.** Any blocking here stalls the entire `/ws/plexus` reader. No `sender_id` fallback scan — direct `chat_id` lookup only. If no match, warn and drop. Cron job outputs are saved to DB by plexus-server regardless; users can see them via `GET /api/sessions/{id}/messages`.

```rust
pub enum RouteResult {
    DirectHit,
    NoMatch,
    Evicted, // browser queue was full on a final message; connection was evicted
}

pub fn route_send(state: &Arc<AppState>, msg: &Value) -> RouteResult {
    // 1. Parse chat_id, session_id, content, metadata, media.
    // 2. Build an OutboundFrame (Progress or Message).
    // 3. Clone the BrowserConnection OUT of the DashMap shard synchronously.
    let conn = state.browsers.get(chat_id).map(|r| r.clone());
    // (shard guard dropped here — no await between get() and clone())

    // 4. Try direct dispatch.
    if let Some(conn) = conn {
        return try_dispatch(state, chat_id, conn, frame);
    }

    // 5. No match — warn and drop.
    tracing::warn!("routing: no browser for chat_id={chat_id}");
    RouteResult::NoMatch
}

fn try_dispatch(state: &Arc<AppState>, chat_id: &str, conn: BrowserConnection, frame: OutboundFrame) -> RouteResult {
    match frame {
        OutboundFrame::Progress(_) => {
            let _ = conn.tx.try_send(frame); // drop on full, ignore error
            RouteResult::DirectHit
        }
        OutboundFrame::Message(_) => {
            match conn.tx.try_send(frame) {
                Ok(()) => RouteResult::DirectHit,
                Err(_) => {
                    tracing::warn!("evicting slow browser chat_id={chat_id}");
                    state.browsers.remove(chat_id);
                    conn.cancel.cancel();  // tears down reader, writer, keepalive
                    RouteResult::Evicted
                }
            }
        }
        OutboundFrame::Error(_) => {
            // Error frames are only constructed by the reader loop, not routing;
            // if one ever reaches here it's a bug — but handle gracefully.
            let _ = conn.tx.try_send(frame);
            RouteResult::DirectHit
        }
    }
}
```

**Never holds a DashMap shard guard across an await.** `route_send` is a synchronous `fn`. The `.clone()` inside `.map(|r| r.clone())` happens while holding the guard synchronously, then the guard is dropped before `try_dispatch` runs.

**Never blocks on a slow consumer.** `try_send` returns immediately. On progress-frame overflow we drop silently. On final-frame overflow we remove the entry and cancel its `CancellationToken`; the writer, keepalive, and reader tasks all exit via the cancel arm of their respective `select!`s within one tokio wake.

### REST Proxy: `/api/*`

Handler lives in `proxy.rs`. Behavior matches `plexus-gateway/docs/PROTOCOL.md`:

- Public endpoints under `/api/auth/*` skip JWT validation (login, register, and any future auth endpoints like password reset).
- All other `/api/*` paths require `Authorization: Bearer <JWT>`, validated at the gateway before proxying.
- Forward method, headers, and body to `{PLEXUS_SERVER_API_URL}{path}`.
- Strip hop-by-hop headers: `host`, `connection`, `transfer-encoding`, `upgrade`, `keep-alive`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailer`.
- Reject path traversal (`..`) with HTTP 422.
- **Max request body:** 25 MB via `tower_http::limit::RequestBodyLimitLayer`. Oversized → HTTP 413.
- **Max response body:** 25 MB, enforced via `reqwest::Response::bytes()` with a length check. If the response exceeds 25 MB, return HTTP 502 with `{"error":{"code":"upstream_too_large","message":"response body exceeded 25 MB limit"}}`. No streaming byte counter — `bytes()` with a limit is sufficient for M3 payload sizes.
- Uses the shared `reqwest::Client` from `AppState`. One pool, many requests.
- Network failure → 502 Bad Gateway with JSON body `{"error":{"code":"upstream_unreachable","message": ...}}`.

### Static Files: `/`

Handler lives in `static_files.rs`. Uses `tower-http::services::ServeDir` rooted at `PLEXUS_FRONTEND_DIR`, with a fallback to `index.html` for any path that doesn't match a file (SPA client-side routing). Registered as the **lowest-priority** route so `/ws/*`, `/api/*`, and `/healthz` always win.

### Health Check: `/healthz`

Handler lives in `health.rs`. Returns HTTP 200 with:

```json
{"status":"ok","plexus_connected":true,"browsers":42}
```

`plexus_connected` = `state.plexus.read().await.is_some()` (short-held read guard). `browsers` = `state.browsers.len()`. Unauthenticated — load balancers need to hit this without a JWT.

### Graceful Shutdown

`lib::run_from_env` installs signal handlers for `SIGTERM` and `SIGINT` (via `tokio::signal`). On signal:

1. `state.shutdown.cancel()` fires. Because every `BrowserConnection.cancel` is a child of `state.shutdown`, all per-connection cancel tokens fire simultaneously. Reader, writer, and keepalive tasks exit cleanly via the cancel arm of their `select!` loops.
2. `axum::serve(listener, app).with_graceful_shutdown(shutdown_future)` — where `shutdown_future` awaits `state.shutdown.cancelled()` and then sleeps for 5 seconds to give in-flight writes a chance to drain. This stops accepting new connections.
3. The axum server future completes once all in-flight requests finish. `run_from_env` returns and the process exits.

**No sleep-based races:** the tokio runtime wakes every `select!` as soon as the parent cancel token fires. The 5-second sleep inside `with_graceful_shutdown` is purely an upper bound for drain; in practice handlers exit within a few milliseconds.

### CORS / Origin

`PLEXUS_ALLOWED_ORIGINS` env var controls both CORS (for REST) and the WS `Origin` check:

- `*` (default) → permissive, CORS allows any origin, WS upgrade does not check `Origin`. Dev only.
- `https://plexus.example.com,https://admin.plexus.example.com` → strict list. Both REST and WS upgrade reject any origin not in the list.

Parsed once at startup. Non-`*` config is required for production deployments; this is called out in DEPLOYMENT.md.

### Error Handling

- **JWT invalid/expired** → HTTP 401 (browsers before WS upgrade, REST proxy at middleware).
- **Origin rejected** → HTTP 403.
- **Wrong `PLEXUS_GATEWAY_TOKEN`** → `auth_fail` + drop.
- **Duplicate plexus connection** → `auth_fail(reason="duplicate connection")` + drop the new one.
- **Plexus not connected, browser sends message** → `{"type":"error","reason":"Plexus server not connected"}` to the browser; connection stays alive.
- **Invalid session_id prefix** → `{"type":"error","reason":"invalid session_id"}`; connection stays alive.
- **Proxy upstream 5xx** → pass through.
- **Proxy network error** → 502 Bad Gateway JSON.
- **Path traversal** → 422.
- **Request body > 25 MB** → 413.
- **Response body > 25 MB** → 502 with `upstream_too_large`.
- **DashMap lookup miss, channel full (progress), etc.** → log warn, drop the message, never panic.
- **Channel full on final frame** → evict the browser, log warn.

### Observability

- `tracing::info_span!("ws_chat", chat_id = %chat_id, user_id = %user_id)` wraps the browser reader loop.
- `tracing::info_span!("ws_plexus")` wraps the plexus reader loop.
- `tracing::info_span!("proxy", method = %method, path = %path)` wraps each proxy request.
- `tracing::warn!` on: slow-browser evictions, routing misses, WS upgrade failures, proxy upstream errors.
- **JWT redaction in access logs:** the integration test setup includes a `tower_http::trace::TraceLayer` with a custom `MakeSpan` that strips the `token` query param from the logged URL. Deployments should configure reverse proxy access logs to do the same.

### Testing

Unit tests (`cargo test --package plexus-gateway`):

- `jwt.rs`: valid token, expired token, malformed token, wrong secret, missing claims.
- `routing.rs`: direct chat_id lookup, no-match, **slow-consumer eviction**, **progress drop on full**.
- Constant-time token comparison: equal-length match, equal-length mismatch, length mismatch without byte comparison.

Integration tests (in-process, ephemeral port, isolated `Config` per test):

- Browser↔plexus round-trip (happy path).
- `send` from plexus → browser receives.
- Browser with no plexus connected → error reply, stays connected.
- Invalid JWT → upgrade rejected (HTTP 401).
- Disallowed `Origin` → upgrade rejected (HTTP 403).
- Duplicate plexus connection → `auth_fail`.
- **Invalid session_id prefix → error frame, connection stays alive.**
- **Progress hint forwarding.**
- **Media attachment forwarding** (browser sends media array → plexus receives it → plexus sends media in reply → browser receives it).
- **Slow browser eviction** (stuff the outbound channel, assert eviction, assert other browsers are unaffected).
- **Reconnect** (browser disconnects and reconnects with same JWT; gateway accepts, issues new `chat_id`).
- **Reader/writer leak test**: connect 200 browsers, disconnect all, assert `state.browsers.len() == 0` within 100ms.
- **REST proxy round-trip** (GET and POST via mock upstream).
- **REST proxy path traversal** rejected with 422.
- **REST proxy request body > 25 MB** rejected with 413.
- **REST proxy response body > 25 MB — Content-Length path** rejected with 502 before the body is read.
- **REST proxy response body > 25 MB — streaming path** (upstream sends chunked without Content-Length) rejected with 502 after the running counter trips.
- **Browser keepalive timeout** — test that connects a browser, never replies to pings, and asserts the connection is closed and evicted within ~50 seconds (30s first ping + 15s pong timeout + slack).
- **`/healthz` returns correct state** (plexus-disconnected and plexus-connected cases).
- **Graceful shutdown** (trigger `state.shutdown.cancel()`, assert server exits within 6s, live browsers receive close frame, reader/writer/keepalive tasks all exit).

### Performance Notes

Target: 1,000 users, 500 concurrent WS sessions. The architecture handles this by design:

- **DashMap** → lock-free concurrent reads, shard-based writes. Browser routing is O(1). Shard guards are never held across awaits (enforced by the "clone out first" rule).
- **Stateless routing** → each message is: parse JSON → DashMap lookup → clone → non-blocking `try_send`. Sub-millisecond.
- **Non-blocking `/ws/plexus` reader** → slow consumers are evicted, never block other browsers.
- **Connection-pooled `reqwest::Client`** → shared across all proxy calls.
- **Bounded per-browser channels** → prevents memory blowup; eviction policy caps per-connection RSS at ~64 × frame size.
- **`LimitNOFILE=65536`** in the systemd unit from DEPLOYMENT.md covers the fd ceiling.
- **App-level ping/pong** → detects dead connections without waiting for TCP timeouts; prevents zombie DashMap entries.

The real load is on plexus-server (LLM calls, DB, tool execution). The gateway is a thin multiplexer.

---

## Phase 2 — plexus-frontend

### Stack

Locked in per `plexus-frontend/docs/DECISIONS.md`:

- React 19, TypeScript 5.9
- Vite 8 (dev server + build)
- Tailwind CSS 4 (via `@tailwindcss/vite`)
- Zustand 5 (state management, no provider wrapping)
- react-router-dom 7 (routing)
- react-markdown + remark-gfm + react-syntax-highlighter (agent output rendering)
- lucide-react (icons)

### Visual Style: Cyberpunk Refined

Per the brainstorming session, the chosen direction is **Cyberpunk Refined** with modifications:

- Base background: `#0d1117` (GitHub dark)
- Sidebar background: `#0a0f18`
- Card/message background: `#161b22`
- Border: `#1a2332`
- Accent green: `#39ff14` (neon)
- Muted text: `#8b949e`
- Primary text: `#e6edf3`
- User message bubble: `bg-[#39ff14]/10`, rounded `12px 12px 2px 12px`
- Agent message bubble: `bg-[#161b22]`, rounded `2px 12px 12px 12px`, `border-l-3 border-[#39ff14]`
- Code blocks inline monospace with green text on darker background
- Device status dots with subtle `box-shadow` glow

**Slim collapsible sidebar** (~1/6 viewport width, 140–200px), collapses to a 48px icon strip. Top-bar shows session name and per-device status dots (server + each device, green for online, red for offline). Input box is responsive — stays between a min and max width so it's neither tiny on 4K nor huge on 13-inch.

### Layout

```
plexus-frontend/
├── package.json
├── vite.config.ts        — proxies /api and /ws to http://localhost:9090
├── tailwind.config.ts
├── tsconfig.json
├── index.html
└── src/
    ├── main.tsx          — router bootstrap
    ├── App.tsx           — route guards (redirect to /login if no JWT)
    ├── lib/
    │   ├── api.ts        — fetch wrapper with JWT header, 401 handling
    │   ├── ws.ts         — WebSocket client with auto-reconnect
    │   └── types.ts      — TypeScript types mirroring server API responses
    ├── store/
    │   ├── auth.ts       — Zustand: token, user, login(), logout()
    │   ├── chat.ts       — Zustand: sessions, messages, progress hints, sendMessage()
    │   └── devices.ts    — Zustand: device list (polled every 5s)
    ├── pages/
    │   ├── Login.tsx     — email/password, calls /api/auth/login
    │   ├── Chat.tsx      — sidebar + message list + input
    │   ├── Settings.tsx  — tabs: Profile / Devices / Channels / Skills / Cron
    │   └── Admin.tsx     — tabs: LLM / Default Soul / Rate Limit / Server MCP
    ├── components/
    │   ├── Sidebar.tsx           — slim session list with collapse toggle
    │   ├── MessageList.tsx       — scrollable message history
    │   ├── Message.tsx           — single message bubble (user or agent)
    │   ├── ProgressHint.tsx      — spinner + ephemeral tool hint text
    │   ├── ChatInput.tsx         — auto-growing textarea, responsive sizing
    │   ├── DeviceStatusBar.tsx   — top-bar dots (server + devices)
    │   └── MarkdownContent.tsx   — react-markdown + syntax highlighting
    └── styles/
        └── globals.css   — Tailwind base + theme CSS vars
```

### Routing

- `/login` — public
- `/` — redirect to `/chat`
- `/chat` and `/chat/:session_id` — requires JWT
- `/settings` — requires JWT
- `/admin` — requires JWT + `is_admin: true` (non-admins get redirected to `/chat`)

`App.tsx` wraps protected routes with a guard that reads `useAuthStore().token`. Any API call that receives 401 clears the token and redirects to `/login`.

### Auth Flow

1. User hits `/` → redirect to `/login` (no token) or `/chat` (has token).
2. User submits email + password → `POST /api/auth/login` → store `{token, user_id, is_admin}` in Zustand + `localStorage` (`jwt` key).
3. All subsequent `fetch` calls include `Authorization: Bearer <token>`.
4. WebSocket connection uses `ws://host/ws/chat?token=<token>`.
5. Logout clears `localStorage` and Zustand, disconnects WebSocket, redirects to `/login`.
6. **Token expiry:** the fetch wrapper (`lib/api.ts`) treats HTTP 401 as a logout trigger. This is the only reliable path. The WS manager does **not** try to detect auth failure — gateway-side JWT validation happens before the WebSocket upgrade, which surfaces in the browser as `onerror` followed by `onclose` with no `onopen`, and this is indistinguishable from a transient network failure. Instead, any API call made by the Chat page (e.g., `GET /api/sessions`, `GET /api/sessions/:id/messages`) will catch the 401 and trigger logout. If the user doesn't make any API calls, the WS will keep reconnecting and the user will see "connecting..." in the device status bar; they can log out manually.

### Session Model (frontend side)

The browser owns session state. This aligns with the gateway protocol (r2).

- **Session ID generation:** `gateway:{user_id}:{crypto.randomUUID()}`. Done in the browser on demand.
- **Current session source of truth:** the URL path `/chat/:sessionId`. The chat store mirrors the URL for ergonomic access; navigation is the write path.
- **Start a new chat:** generate a new session_id locally, navigate to `/chat/:newSessionId`. No server roundtrip. The session row is created on plexus-server when the first message is sent.
- **Switch sessions:** click a sidebar entry → navigate to `/chat/:otherSessionId`. Chat store listens to URL changes and loads history.
- **Resume on reload:** the URL has the session_id, the chat store reads it, loads history via REST, and opens the WS.
- **Open in new tab:** works automatically — each tab has its own URL.

**There is no `new_session` / `switch_session` / `session_created` / `session_switched` message type.** These were removed from PROTOCOL.md r2.

### WebSocket Lifecycle

`lib/ws.ts` exports a singleton WebSocket manager. States: `connecting | open | closed`.

- **Connect:** called from the Chat page mount effect with the current JWT. Idempotent — calling `connect(token)` when already connected with the same token is a no-op.
- **URL:** derived from `window.location`: `${protocol}//${host}/ws/chat?token=${token}`.
- **Reconnect:** exponential backoff with jitter. Base delays `1s, 2s, 4s, 8s, 16s, 30s`, each multiplied by `(0.75 + Math.random() * 0.5)` to spread reconnect stampedes after a gateway restart. Max 30s cap.
- **Ping:** the browser responds to gateway `{"type":"ping"}` with `{"type":"pong"}`. No client-initiated pings (the gateway is authoritative for liveness).
- **Message dispatch:** parse JSON, dispatch by `type`:
  - `message` → `chat.handleIncomingMessage(sessionId, content, media)`. Appends to that session's message list; no dedup against REST history (see Chat Store below).
  - `progress` → `chat.setProgressHint(sessionId, content)`.
  - `error` → `chat.handleError(reason)`. Shows a toast; does not trigger logout (logout is triggered by REST 401).
  - `ping` → reply with `pong` immediately.
  - Unknown → log warn, ignore.
- **Listener management:** `onMessage(fn)` and `onStatus(fn)` return an unsubscribe function. The chat store's `init()` is idempotent (guarded by module-scoped flags) and stores the unsubscribe handles in module-level variables so React StrictMode double-invocations don't leak listeners.
- **On close:** always trigger reconnect with jitter (no "auth_failed" terminal state — if the JWT is invalid, reconnect will keep failing and the user will eventually trigger a REST call that surfaces the 401).
- **Disposed on logout.**

### Chat Store Contract

`store/chat.ts` holds:

```ts
interface ChatState {
  sessions: Session[]
  currentSessionId: string | null
  messagesBySession: Record<string, ChatMessage[]>
  restLoadedSessions: Set<string>  // tracks which sessions have already been loaded via REST
  progressBySession: Record<string, string | null>
  wsStatus: 'connecting' | 'open' | 'closed'

  // mutations
  init: () => void                                        // idempotent; attaches WS listeners once
  loadSessions: () => Promise<void>
  loadMessages: (sessionId: string) => Promise<void>      // no-op if already loaded
  setCurrentSession: (sessionId: string | null) => void
  sendMessage: (sessionId: string, content: string, media?: string[]) => void
  handleIncomingMessage: (sessionId: string, content: string, media?: string[]) => void
  setProgressHint: (sessionId: string, hint: string) => void
  clearProgress: (sessionId: string) => void
  handleError: (reason: string) => void
}
```

**Initial-load merge semantics (no clobber, but also no cross-source dedup):**
- `loadMessages` is guarded by `restLoadedSessions`: calling it twice for the same session is a no-op. This prevents duplicate-render when the user switches away and back.
- On first call for a session: `loadMessages` fetches `GET /api/sessions/:id/messages?limit=200`, builds a `ChatMessage[]`, **prepends** it to any messages already in `messagesBySession[sessionId]`, sorts by `created_at`, and adds the session to `restLoadedSessions`. "Prepending" means: any WS messages that arrived during the REST fetch are preserved (no clobber), and the REST snapshot represents the history before those WS messages.
- WS messages use client-generated `id = crypto.randomUUID()`; REST messages use server `message_id`. We do **not** claim to dedup the same assistant message across WS+REST, because a WS final reply that also appears in REST history would render twice. In practice this cannot happen in M3 because `loadMessages` is only called once per session (on first mount), which is before any WS replies for that session exist.
- Optimistic local echo on `sendMessage` uses `crypto.randomUUID()`; it is visible immediately and is replaced-in-place when the server confirms via WS (TODO: in M4, once server emits stable message_ids on WS finals, we can dedup properly).

**Progress hint lifecycle:**
- Set by incoming `progress` frames: `progressBySession[sessionId] = content`.
- Cleared when a final `message` arrives for the same session: `progressBySession[sessionId] = null`.
- Cleared when the user navigates to a different session.
- Not persisted to localStorage. Fresh page load = no hints (correct: they are ephemeral and the agent may no longer be running).

### Chat Page Layout

Two states, keyed off `messagesBySession[currentSessionId]?.length`:

**Empty state (no messages in this session):**
- Sidebar (slim) on the left.
- Center: greeting ("Hey, Yucheng"), input box mid-screen.
- Responsive: input stays between `min(90vw, 420px)` and `min(90vw, 720px)` — never tiny, never huge.

**Active state (messages present):**
- Sidebar (slim) on the left.
- Top bar: session name + device status dots.
- Message list fills the middle, scrolls.
- Progress hint (if active) shows at the bottom of the list, above the input.
- Input drops to bottom, same responsive width as empty state.

**URL-driven session:** `Chat.tsx` reads `useParams<{ sessionId: string }>()`. On mount or URL change:
1. If `sessionId` is missing from URL → generate one with `crypto.randomUUID()` and `navigate('/chat/:newId', { replace: true })`.
2. If `!messagesBySession[sessionId]` → `loadMessages(sessionId)`.
3. Ensure WS is connected (idempotent `wsManager.connect(token)`).
4. Register `beforeunload` to call `wsManager.disconnect()` on logout only — not on navigation.

**New chat button:** generates a UUID and navigates — zero server roundtrip. The session row is created on plexus-server when the first message is sent.

**Switch session:** sidebar button navigates. `Chat.tsx` effect runs `loadMessages` and updates `progressBySession` (clears the hint for the old session, since we don't know if it's still running).

**Session history on page load:** `Chat.tsx` calls `GET /api/sessions` for the sidebar and `GET /api/sessions/{id}/messages?limit=200` for the current session. Paginated (50 default, 200 explicit cap).

### Settings Page

Tabs inside a single page, each tab loads its own data lazily.

**Profile tab:**
- Display email, user_id, admin status (read-only from `GET /api/user/profile`)
- Soul editor: textarea, `GET/PATCH /api/user/soul`
- Memory editor: textarea with 4K char counter, `GET/PATCH /api/user/memory`

**Devices tab:**
- List devices from `GET /api/devices` (polled every 5s): device name, online/offline dot, tool count, last seen.
- Token management: `GET /api/device-tokens`, `POST /api/device-tokens` (create), `DELETE /api/device-tokens/{token}` (with copy-to-clipboard on create).
- Per-device expand panel:
  - Sandbox policy: `GET/PATCH /api/devices/{name}/policy` — dropdown between `sandbox` and `unrestricted`.
  - MCP config: `GET/PUT /api/devices/{name}/mcp` — JSON editor.

**Channels tab** (per user request):
- Subsection: Discord — `GET/POST/DELETE /api/discord-config`. Form fields: bot token (password input), allowed users (tag input), owner Discord ID.
- Subsection: Telegram — `GET/POST/DELETE /api/telegram-config`. Form fields: bot token, partner telegram ID, allowed users, group policy (dropdown: `mention` | `all`).

**Skills tab:**
- List skills from `GET /api/skills` (current user only — privacy preserved, we removed the admin overview).
- Install from GitHub: form with `repo` and optional `branch`, posts to `/api/skills/install`.
- Paste SKILL.md content: textarea, posts to `/api/skills`.
- Delete button per row, `DELETE /api/skills/{name}`.

**Cron tab:**
- List cron jobs from `GET /api/cron-jobs`.
- Create form: message, schedule (radio: cron_expr / every_seconds / at), channel, name, timezone.
- Enable/disable toggle per row, `PATCH /api/cron-jobs/{id}`.
- Delete button per row, `DELETE /api/cron-jobs/{id}`.

### Admin Page

Only accessible to users with `is_admin: true` in JWT claims.

**LLM tab:**
- `GET /api/llm-config` → show masked API key.
- Edit form: api_base, model, api_key (password input), context_window.
- Save → `PUT /api/llm-config`.

**Default Soul tab:**
- `GET /api/admin/default-soul` → textarea.
- Save → `PUT /api/admin/default-soul`.

**Rate Limit tab:**
- `GET /api/admin/rate-limit` → number input.
- Save → `PUT /api/admin/rate-limit`.

**Server MCP tab:**
- `GET /api/server-mcp` → JSON editor.
- Save → `PUT /api/server-mcp` (triggers server-side reinitialization).

### Testing

- `tsc -b` type-check as the primary correctness signal.
- Vitest component smoke tests for `ChatInput`, `Message`, `MarkdownContent`, `ProgressHint`.
- Vitest store tests for `chat.ts` covering:
  - REST/WS race: `loadMessages` followed by `handleIncomingMessage` during the load — the WS message must not be clobbered.
  - Dedup: loading the same REST page twice does not duplicate messages.
  - Progress hint cleared on final message.
  - Progress hint cleared on session switch.
  - Idempotent `init()` — calling twice registers listeners once.
- No Playwright in M3 — manual end-to-end validation is the final gate.

---

## Build & Deployment

### Workspace Update

Add `plexus-gateway` to `Cargo.toml` workspace members:

```toml
[workspace]
members = ["plexus-common", "plexus-client", "plexus-server", "plexus-gateway"]
```

### Dev Workflow

Three terminals:

1. `cargo run --package plexus-server` (port 8080)
2. `cargo run --package plexus-gateway` (port 9090)
3. `cd plexus-frontend && npm run dev` (port 5173, Vite proxy to 9090)

Open http://localhost:5173. Hot-reload works on the frontend; Rust crates require a rebuild on change.

### Production Workflow

1. `cd plexus-frontend && npm ci && npm run build` → `plexus-frontend/dist/`
2. `cargo build --release --package plexus-gateway` → `target/release/plexus-gateway`
3. Edit gateway `.env` with `PLEXUS_FRONTEND_DIR=./plexus-frontend/dist` (or absolute path)
4. Run `./plexus-gateway`
5. Open `http://localhost:9090` — chat, settings, admin all served from one process

Single-binary deployment story, just as you wanted.

---

## Open Items After This Spec

- The implementation plan will break Phase 1 and Phase 2 into ordered steps with verification gates.
- `plexus-server/docs/API.md` has been updated this session with the missing Telegram section.
- `GET /api/admin/skills` has been removed from the codebase, docs, and DB layer.

## Related Documents

- `plexus-gateway/docs/DECISIONS.md` — architecture rationale (frozen)
- `plexus-gateway/docs/PROTOCOL.md` — WebSocket wire format (frozen)
- `plexus-gateway/docs/DEPLOYMENT.md` — deployment recipes (frozen)
- `plexus-frontend/docs/DECISIONS.md` — stack rationale (frozen)
- `plexus-frontend/docs/DEPLOYMENT.md` — frontend build/serve (frozen)
- `plexus-server/docs/API.md` — REST API reference (updated this session)
