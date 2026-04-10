# M3 — Gateway + Frontend Design

**Status:** Design approved, ready for implementation plan
**Date:** 2026-04-10
**Scope:** Build `plexus-gateway` (Rust) and `plexus-frontend` (React) from scratch

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
    ├── routing.rs        — chat_id → browser lookup, sender_id fallback
    └── ws/
        ├── mod.rs        — shared WS types (BrowserConnection, PlexusConnection)
        ├── chat.rs       — /ws/chat (browser WS handler)
        └── plexus.rs     — /ws/plexus (server WS handler, constant-time token)
```

### Dependencies

```toml
axum = { version = "0.7", features = ["ws"] }
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.6", features = ["cors", "fs", "trace", "limit"] }
futures-util = "0.3"
jsonwebtoken = "9"
subtle = "2"
dashmap = "6"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
plexus-common = { path = "../plexus-common" }

[dev-dependencies]
tokio-tungstenite = "0.24"  # only for integration test clients
```

`axum`'s built-in `ws` feature handles server-side WebSocket upgrades. `tokio-tungstenite` is only pulled in for integration tests that need a client to dial into the gateway. `plexus-common` is reused for error types (`ApiError`, `ErrorCode`) — no separate `error.rs` in the gateway.

### State

The gateway is deliberately **stateless** beyond what the live WebSocket topology requires:

Each browser connection spawns a dedicated writer task that owns the sink directly — other tasks send outbound frames through a bounded channel. This gives us natural per-connection backpressure (see the Backpressure subsection below) and avoids mutex contention on the sink.

```rust
pub struct AppState {
    pub config: Config,
    pub browsers: Arc<DashMap<String, BrowserConnection>>,         // chat_id → per-connection handle
    pub plexus:   Arc<RwLock<Option<tokio::sync::mpsc::Sender<serde_json::Value>>>>,  // single server sender
    pub http_client: reqwest::Client,                              // pooled, shared across all proxy requests
}

pub struct BrowserConnection {
    pub outbound: tokio::sync::mpsc::Sender<OutboundFrame>,        // bounded channel to writer task
    pub user_id: String,                                           // from JWT; needed for sender_id fallback
}

pub enum OutboundFrame {
    Message(serde_json::Value),  // final message — must deliver or drop connection
    Progress(serde_json::Value), // ephemeral — may be dropped under backpressure
}
```

**Session state is not held at the gateway.** The browser sends `session_id` with every message (per PROTOCOL.md); the server is the DB-backed source of truth for session lifecycle. The gateway generates a new `session_id = "gateway:{user_id}:{uuid}"` on connect or on `new_session`, echoes it back, and otherwise passes whatever the browser sends. This makes gateway restarts trivially safe — no session state is lost because the gateway never owned it.

### Environment Variables

| Variable | Required | Default | Source |
|---|---|---|---|
| `PLEXUS_GATEWAY_TOKEN` | yes | — | shared secret for plexus-server auth |
| `JWT_SECRET` | yes | — | HMAC secret, must match server |
| `GATEWAY_PORT` | yes | — | listen port |
| `PLEXUS_SERVER_API_URL` | yes | — | upstream base URL for REST proxy |
| `PLEXUS_FRONTEND_DIR` | no | `./plexus-frontend/dist` | static file root |

`dotenvy` loads a `.env` file in the working directory at startup.

### WebSocket: `/ws/chat` (browsers)

Handler lives in `ws/chat.rs`. Flow:

1. Extract `token` query parameter (JWTs cannot use the `Authorization` header on WS upgrade in browsers).
2. Validate via `jwt::validate()` using `JWT_SECRET`. Expected claims: `{ sub: String, is_admin: bool, exp: u64 }`.
3. On failure → return HTTP 401 *before* upgrade. No resources allocated.
4. On success → upgrade, generate `chat_id = UUIDv4()` and `session_id = format!("gateway:{}:{}", user_id, uuid::new_v4())`.
5. Insert into `state.browsers[chat_id] = BrowserConnection { sink, user_id }`.
6. Send `{"type":"session_created","session_id": ...}` immediately.
7. Enter read loop. Dispatch by `type`:
   - `message` → call `routing::forward_to_plexus(state, chat_id, user_id, content, media, session_id)`. If the plexus sink is `None`, send `{"type":"error","reason":"Plexus server not connected"}` back to this browser.
   - `new_session` → generate fresh `session_id`, send `session_created`.
   - `switch_session` → echo `{"type":"session_switched","session_id": ...}` back.
   - Unknown type → log warn, ignore.
8. On disconnect → remove `chat_id` from `state.browsers`.

**Per-connection backpressure.** Each browser gets a bounded `tokio::sync::mpsc::channel(64)` between the router and its sink writer task. If the queue is full:
- For progress hints: drop the oldest (progress is ephemeral).
- For final `message` frames: drop the connection (slow client cannot keep up).

This prevents one stuck browser from ballooning gateway memory under load.

### WebSocket: `/ws/plexus` (plexus-server, exactly one)

Handler lives in `ws/plexus.rs`. Flow:

1. Upgrade immediately — no query auth.
2. Wait for the first text frame (with a 5-second timeout).
3. Parse as `{"type":"auth","token": "..."}`. Any other shape → `auth_fail` + drop.
4. Compare `token` to `PLEXUS_GATEWAY_TOKEN` using `subtle::ConstantTimeEq::ct_eq()` on equal-length byte slices. Length mismatch short-circuits to false (length is not a secret).
5. Acquire `state.plexus.write().await`. If already `Some` → `auth_fail` (`reason: "duplicate connection"`) + drop. Otherwise store the sink.
6. Send `{"type":"auth_ok"}`.
7. Enter read loop. Handle `send` messages:
   - Look up `chat_id` in `state.browsers`. If found → forward.
   - Fallback: if not found, look up by `metadata.sender_id` and route to *any* open browser for that user (handles cron-triggered pushes when the original `chat_id` is stale).
   - If neither → log warn, drop silently.
   - If `metadata._progress == true` → emit as `{"type":"progress",...}`. Otherwise emit as `{"type":"message",...}`. Include `session_id` (from the upstream message) and `media` (from `metadata.media`) when present.
8. On disconnect → clear `state.plexus`. No browser impact — browsers will just get "server not connected" errors on their next message.

### REST Proxy: `/api/*`

Handler lives in `proxy.rs`. Behavior matches `plexus-gateway/docs/PROTOCOL.md`:

- Public endpoints `/api/auth/login` and `/api/auth/register` skip JWT validation.
- All other paths require `Authorization: Bearer <JWT>`, validated at the gateway before proxying.
- Forward method, headers, and body to `{PLEXUS_SERVER_API_URL}{path}`.
- Strip hop-by-hop headers: `host`, `connection`, `transfer-encoding`, `upgrade`, `keep-alive`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailer`.
- Reject path traversal (`..`) with 422.
- Max body: 25 MB (request and response). Enforced via `tower::limit::RequestBodyLimitLayer` and streamed response copy.
- Uses the shared `reqwest::Client` from `AppState`. One pool, many requests.
- Network failure → 502 Bad Gateway with JSON body `{"error":{"code":"upstream_unreachable","message": ...}}`.

### Static Files: `/`

Handler lives in `static_files.rs`. Uses `tower-http::services::ServeDir` rooted at `PLEXUS_FRONTEND_DIR`, with a fallback to `index.html` for any path that doesn't match a file (SPA client-side routing). Registered as the **lowest-priority** route so `/ws/*` and `/api/*` always win.

### CORS

Permissive CORS via `tower-http::cors`:

```rust
CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any)
```

Per DEPLOYMENT.md, this is safe because the gateway is behind a reverse proxy in production. For direct exposure, operators can tighten via reverse proxy.

### Error Handling

- **JWT invalid/expired** → 401 (browsers), `ApiError` JSON (REST).
- **Wrong `PLEXUS_GATEWAY_TOKEN`** → `auth_fail` + drop.
- **Duplicate plexus connection** → `auth_fail(reason="duplicate connection")` + drop the new one.
- **Plexus not connected, browser sends message** → `{"type":"error","reason":"Plexus server not connected"}` to the browser; connection stays alive.
- **Proxy upstream 5xx** → pass through.
- **Proxy network error** → 502 Bad Gateway JSON.
- **Path traversal** → 422.
- **Body > 25 MB** → 413.
- **DashMap lookup miss, channel full, etc.** → log warn, drop the message, never panic.

### Testing

Unit tests (`cargo test --package plexus-gateway`):

- `jwt.rs`: valid token, expired token, malformed token, wrong secret, missing `sub`.
- `routing.rs`: direct chat_id lookup, sender_id fallback, no-match drop.
- Constant-time token comparison: equal-length match, equal-length mismatch, length mismatch returns false without byte comparison.

Integration tests (in-process, using `axum::serve` on ephemeral port):

- Start the gateway, open a mock plexus WS client, authenticate, send `send` messages and assert the browser mock receives them.
- Open a mock browser WS client with a valid JWT, send `message` and assert the mock plexus receives it.
- Assert the error flow: browser connects, no plexus, browser sends message → gets error reply, stays connected.
- Assert REST proxy: mock upstream HTTP server, gateway proxies a GET and POST, headers and body match.

### Performance Notes

Target: 1,000 users, 500 concurrent WS sessions. The architecture handles this by design:

- **DashMap** → lock-free concurrent reads, shard-based writes. Browser routing is O(1) with no cross-shard contention.
- **Stateless routing** → each message is: parse JSON → DashMap lookup → forward bytes. Sub-millisecond.
- **Connection-pooled `reqwest::Client`** → shared across all proxy calls.
- **Bounded per-browser channels** → prevents slow-client memory blowup.
- **`LimitNOFILE=65536`** in the systemd unit from DEPLOYMENT.md covers the fd ceiling.

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

### WebSocket Lifecycle

`lib/ws.ts` exports a singleton WebSocket manager:

- Connects on first use (when Chat page mounts).
- URL derived from `window.location`: `${protocol}//${host}/ws/chat?token=${token}`.
- Auto-reconnect with exponential backoff (1s, 2s, 4s, 8s, 16s, cap at 30s).
- On open: emit `connected` event to the chat store.
- On message: parse JSON, dispatch by `type`:
  - `session_created` / `session_switched` → update current session in chat store.
  - `message` → append to message list for that session; clear progress hint for that session.
  - `progress` → set progress hint for that session (ephemeral).
  - `error` → show toast, do not append to history.
- On close: trigger reconnect, emit `disconnected` event.
- Disposed on logout.

### Chat Page Layout

Two states:

**Empty state (new session, no messages):**
- Sidebar (slim) on the left.
- Center: greeting ("Hey, Yucheng" or similar), input box mid-screen.
- Responsive: input stays between `min(90vw, 420px)` and `min(90vw, 720px)` — never tiny, never huge.

**Active state (messages present):**
- Sidebar (slim) on the left.
- Top bar: session name + device status dots.
- Message list fills the middle, scrolls.
- Progress hint (if active) shows at the bottom of the list, above the input.
- Input drops to bottom, same responsive width as empty state.

**Tool progress hint rendering:** ephemeral green spinner + `"Executing shell on laptop..."` text (the exact string comes from the server's `build_tool_hint()`). Not persisted. Cleared when a final `message` arrives, when the user switches sessions, and on page reload (fresh history from `/api/sessions/{id}/messages` has no hints by design).

**Session history on page load:** `Chat.tsx` calls `GET /api/sessions` for the sidebar and `GET /api/sessions/{id}/messages` for the current session. Messages are paginated (50 per page by default).

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
- No Playwright in M3 — manual validation in the browser.

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
