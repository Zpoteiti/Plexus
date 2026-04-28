# Gateway WebSocket Protocol

All messages are JSON over WebSocket. Discriminated by `"type"` field. Two independent WebSocket endpoints:

- `/ws/chat` -- browser clients
- `/ws/plexus` -- plexus-server (exactly one connection allowed)

REST calls to `/api/*` are JWT-validated and reverse-proxied to plexus-server (see proxy section below).

## Session Model

**The browser owns session state.** Every inbound `message` must carry a `session_id` generated client-side. The gateway is stateless with respect to sessions — it forwards messages, it does not track "current session per chat_id" or emit `session_created` / `session_switched` acknowledgements.

**Session ID format:** `gateway:{user_id}:{uuid}`. The browser generates these locally using `crypto.randomUUID()`, where `user_id` is the JWT `sub` claim. To start a new conversation, the browser just picks a new ID. To resume an old one, it uses the existing ID (typically read from the URL path `/chat/:session_id`).

**Gateway ownership enforcement:** the gateway validates that every inbound `session_id` starts with `gateway:{user_id}:` where `user_id` matches the JWT `sub`. Any message with a mismatched prefix is rejected with an `error` frame and the connection stays open. This prevents cross-user session spoofing.

**Server side:** plexus-server treats `session_id` the same as it does for Discord and Telegram channels — it auto-creates the DB row on first use. Session ownership on the REST side (`GET /api/sessions/{id}/messages`) is separately enforced by plexus-server against the JWT `sub`.

---

## Browser -> Gateway (`/ws/chat`)

### `message` -- send a chat message

Every message carries a `session_id`. The browser is responsible for generating and persisting session IDs across reloads (typically via URL).

```json
{
  "type": "message",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "hello world"
}
```

With media attachments (optional array of file reference strings):

```json
{
  "type": "message",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "check this out",
  "media": ["file1:screenshot.png"]
}
```

### `pong` -- keepalive response

Replies to a gateway ping. The gateway uses application-level ping/pong (not the WebSocket-layer control frames) so reverse proxies that strip control frames don't break liveness detection.

```json
{"type": "pong"}
```

---

## Gateway -> Browser

### `message` -- agent reply (final)

```json
{
  "type": "message",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "Here's what I found..."
}
```

With media (omitted from JSON when `None`):

```json
{
  "type": "message",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "Here's the image",
  "media": ["https://files.example.com/result.png"]
}
```

### `progress` -- ephemeral tool/thinking hint

Not persisted. The frontend clears the progress state when a final `message` arrives for the same session, when the user switches sessions, or on page reload.

```json
{
  "type": "progress",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "Executing shell on laptop..."
}
```

### `error` -- something went wrong

Reasons include `"Plexus server not connected"`, `"invalid session_id"`, and `"rate limited"`. Does not close the connection.

```json
{"type": "error", "reason": "Plexus server not connected"}
```

### `ping` -- liveness check

Gateway sends these every 30 seconds. Browser must reply with `pong` within 15 seconds or the connection is closed.

```json
{"type": "ping"}
```

---

## Gateway -> Plexus Server (`/ws/plexus`)

Sent when a browser user sends a message. Wraps the browser message with routing metadata.

### `message` -- forwarded user message

`session_id` is passed through exactly as the browser supplied it (after prefix validation).

```json
{
  "type": "message",
  "chat_id": "e3f4a5b6-...",
  "sender_id": "user42",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "hello world"
}
```

With media:

```json
{
  "type": "message",
  "chat_id": "e3f4a5b6-...",
  "sender_id": "user42",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "check this",
  "media": ["file1:screenshot.png"]
}
```

### `auth_ok` -- server auth succeeded

```json
{"type": "auth_ok"}
```

### `auth_fail` -- server auth rejected

```json
{"type": "auth_fail", "reason": "invalid token"}
```

---

## Plexus Server -> Gateway (`/ws/plexus`)

### `auth` -- first message, must be sent immediately on connect

```json
{"type": "auth", "token": "your-gateway-token-here"}
```

### `send` -- push a message to a browser

Every `send` must include a `session_id` — it tells the browser which conversation the message belongs to, which matters because the browser may have switched to a different session while the agent was working.

```json
{
  "type": "send",
  "chat_id": "e3f4a5b6-...",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "Here's your answer"
}
```

With metadata (progress indicator, media, fallback routing):

```json
{
  "type": "send",
  "chat_id": "e3f4a5b6-...",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "thinking...",
  "metadata": {"_progress": true}
}
```

```json
{
  "type": "send",
  "chat_id": "e3f4a5b6-...",
  "session_id": "gateway:user42:a1b2c3d4",
  "content": "done",
  "metadata": {"sender_id": "user42", "media": ["https://files.example.com/out.png"]}
}
```

---

## Auth Flows

### Browser auth (`/ws/chat`)

1. Browser opens `ws(s)://host/ws/chat?token=<JWT>` (browsers cannot set the `Authorization` header on WebSocket upgrade, so the query parameter is the only option).
2. Gateway validates the JWT against `JWT_SECRET` using the `jsonwebtoken` crate. HS256, standard `exp` enforcement.
3. Expected claims: `{ "sub": "user42", "is_admin": false, "exp": 1234567890 }`.
4. Gateway also validates the `Origin` header against the configured allow-list (env var `PLEXUS_ALLOWED_ORIGINS`, comma-separated; default `*` for dev). Missing or disallowed origins are rejected with HTTP 403.
5. On auth success: connection upgrades. No `session_created` is sent — the browser is responsible for its own session state.
6. On auth failure: HTTP 401 returned before upgrade; no resources allocated.
7. Access logs should be configured to redact the `token` query parameter.

### Plexus server auth (`/ws/plexus`)

1. Server opens `ws://host/ws/plexus` (no auth on upgrade).
2. Gateway allocates resources and waits up to 5 seconds for the first text frame.
3. First message must be `{"type": "auth", "token": "..."}`. Any other shape → `auth_fail` + drop.
4. Token compared against `PLEXUS_GATEWAY_TOKEN` using `subtle::ConstantTimeEq`. Length mismatch is a constant-time fail.
5. Only one plexus connection is allowed at a time. Duplicate connections are rejected with `auth_fail(reason="duplicate connection")`.
6. On success: `auth_ok` is sent back; the connection is stored in `state.plexus`.
7. On failure: `auth_fail` is sent and the connection is dropped immediately.

---

## Message Routing

Each browser connection is assigned a random `chat_id` (UUIDv4) at upgrade time. The gateway holds `DashMap<chat_id, BrowserConnection>`; each `BrowserConnection` has a bounded `mpsc::channel(64)` and a dedicated writer task that owns the WebSocket sink.

When plexus-server sends a `send` message:

1. **Direct lookup**: `chat_id` looked up in the DashMap. The entry is cloned out of the shard **before** any await — the shard guard is never held across an async boundary.
2. **Fallback**: if `chat_id` is not found, the gateway scans for any browser whose `user_id` matches `metadata.sender_id`. This handles cron-triggered pushes and cases where the original `chat_id` has expired.
3. **Progress vs final:** `metadata._progress == true` → emit as `progress` type; otherwise emit as `message` type.
4. **Media extraction:** `metadata.media` array is forwarded to the browser when present.
5. **Non-blocking delivery:**
   - Progress frames: `try_send` — on full, the incoming progress frame is dropped silently. Progress is ephemeral and the user already has a recent hint on screen.
   - Final frames: `try_send` — on full, the browser is evicted: the gateway removes the entry from its `DashMap` and cancels the per-connection `CancellationToken`, which triggers the reader, writer, and keepalive tasks to exit. A slow final-message consumer cannot head-of-line block the `/ws/plexus` reader loop.
6. **No match:** warning logged, message dropped silently.

---

## Keepalive & Liveness

**Browser ↔ Gateway:** the gateway sends application-level `{"type":"ping"}` frames every 30 seconds and expects `{"type":"pong"}` within 15 seconds. On timeout, the gateway cancels the per-connection `CancellationToken`, which causes the reader, writer, and keepalive tasks to exit and the `DashMap` entry to be removed. Application-level frames are used (not WebSocket control frames) because some reverse proxies strip control frames.

Pings are sent through the same bounded `mpsc::channel(64)` as data frames, as an `OutboundFrame::Ping` variant. The keepalive task tracks a missed-pong counter (`AtomicU32`); if the counter exceeds 3 (~2 min of silence) or `try_send` fails (channel full), the per-connection `CancellationToken` is cancelled and all tasks exit.

**Plexus-server ↔ Gateway:** no application-level keepalive in M3. The TCP keepalive and the plexus-server's own reconnect loop (1s → 30s exponential backoff) are relied on to detect dead connections. If operational experience shows this is insufficient, app-level keepalive can be added in a later milestone.

---

## Connection Backpressure

Each browser connection has two queue thresholds:

- **Inbound (browser → gateway → plexus):** the gateway forwards one browser message at a time, blocking on the plexus sender channel. The plexus sender has buffer 256. If plexus-server cannot keep up, browser sends block naturally — fine, it's a 1-to-many fan-in.
- **Outbound (plexus → gateway → browser):** each browser has an outbound channel of buffer 64. Progress frames drop newest on overflow (`try_send` drops the incoming frame, not queued ones); final frames trigger eviction. This isolates slow consumers from one another.

The plexus `/ws/plexus` reader loop **must** be non-blocking per message — any blocking call there stalls every browser's outbound flow.

---

## Graceful Shutdown

On `SIGTERM` / `SIGINT`:

1. Stop accepting new connections (close the TCP listener).
2. Send a WebSocket `Close` frame with status 1001 ("going away") to every live browser connection.
3. Drain outbound queues with a 5-second grace window.
4. Close the plexus connection after the browser connections settle.
5. Exit the process.

---

## Health Check

`GET /healthz` returns HTTP 200 with a JSON body:

```json
{"status":"ok","plexus_connected":true,"browsers":42}
```

Unauthenticated. Intended for load balancer readiness probes. The `plexus_connected` field lets operators distinguish "gateway is up but plexus is down" from "gateway is healthy".

---

## REST Proxy (`/api/*`)

All `/api/*` requests are reverse-proxied to plexus-server (`PLEXUS_SERVER_API_URL`).

- **Public endpoints** (no JWT required): `/api/auth/login`, `/api/auth/register`.
- **All other paths** require `Authorization: Bearer <JWT>` validated at the gateway.
- Path traversal blocked (`..` rejected with HTTP 422).
- **Max request body:** 25 MB, enforced via `tower_http::limit::RequestBodyLimitLayer`. Oversized requests return HTTP 413.
- **Max response body:** 25 MB, enforced by streaming the upstream response and aborting if the running total exceeds the limit. A `Content-Length` header exceeding 25 MB is rejected before the body is read.
- Hop-by-hop headers stripped (`host`, `connection`, `transfer-encoding`, `upgrade`, `keep-alive`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailer`).
- Uses a connection-pooled `reqwest::Client` shared across all requests.
- Network failure → 502 Bad Gateway with JSON body `{"error":{"code":"upstream_unreachable","message":...}}`.

---

## Rate Limiting

Rate limiting at the gateway edge is **not implemented in M3**. It is acknowledged as a gateway concern in `DECISIONS.md` and is tracked as M4 scope. Plexus-server has its own per-user token-bucket that covers abuse prevention for the agent loop; the gateway-level limiter is for WS connect churn and REST proxy thrash, which are not current bottlenecks.
