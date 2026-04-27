# Plexus — Device WebSocket Protocol

The wire protocol between `plexus-server` and `plexus-client`. Single connection per device carries both control plane (JSON text frames) and bulk plane (binary frames). Headline decisions are fixed in **ADR-096**; this doc is the operational spec.

Browser ↔ server uses REST + SSE (ADR-003). This protocol is for devices only.

---

## 1. Connection lifecycle

### 1.1 Endpoint

```
GET /ws/device
Authorization: Bearer <PLEXUS_DEVICE_TOKEN>
```

Or (for clients that can't set headers on WS upgrade): `GET /ws/device?token=<...>`.

The token is the device row's primary key (ADR-091, ADR-097). No additional handshake credentials.

### 1.2 Handshake

After the WS upgrade succeeds, **the client sends `hello` first**:

```jsonc
{
  "type": "hello",
  "id": "0190d5a7-...",          // UUID v7, used to correlate hello_ack
  "version": "1",                // protocol version
  "client_version": "0.3.0",     // plexus-client crate version
  "os": "linux",                 // "linux" | "darwin" | "windows" | "android"
  "caps": {                      // what the client can actually do
    "sandbox": "bwrap",          // "bwrap" | "sandbox-exec" | "none"
    "exec": true,
    "fs": "rw"
  }
}
```

Server responds with `hello_ack` containing the device's **server-side configuration** (so the client doesn't need to know workspace_path, ssrf_whitelist, etc. before this point):

```jsonc
{
  "type": "hello_ack",
  "id": "<same as hello.id>",
  "device_name": "alice-laptop",
  "user_id": "...",              // for logs only
  "config": {
    "workspace_path": "/home/alice/.plexus/",
    "fs_policy": "sandbox",
    "shell_timeout_max": 300,
    "ssrf_whitelist": ["10.180.20.30:8080"],
    "mcp_servers": { "minimax": { ... } }
  }
}
```

If the token is invalid or revoked, the server closes with WS code `4401` and a JSON close-reason payload `{"code":"unauthorized"}`. No `error` frame.

### 1.3 Reconnect

On disconnect, the client retries with exponential backoff (1s, 2s, 4s, ..., capped at 30s, jitter ±20%). Each reconnect re-sends `hello` with a **new** `id`. `hello` is idempotent — the server treats reconnect as a fresh session for that device row.

**Initial connect uses the same backoff and never gives up** (ADR-104). If the very first handshake fails (DNS error, TCP refused, TLS failure, 4xx response), the client logs each attempt to stderr and keeps retrying. Only `SIGTERM` / `SIGINT` / OS shutdown stops it. Pairs cleanly with systemd / launchd / Windows service supervision — temporary server downtime doesn't kill the daemon.

In-flight tool calls at the time of disconnect do NOT resume on reconnect. The server has already failed them with `device_unreachable` (see §3.4).

### 1.4 Heartbeat

```jsonc
{ "type": "ping", "id": "..." }
{ "type": "pong", "id": "<echoes ping.id>" }
```

- Server sends `ping` every **30 seconds**.
- Client must respond within 30s of the next `ping` deadline.
- After **2 missed pongs (~70s)** the server closes the connection (WS code `4408`) and marks the device offline. Any in-flight tool calls fail with `device_unreachable` (§3.4).

Either side MAY send `ping` for liveness checks; the responder always echoes `id`.

---

## 2. Frame catalog

All control frames are **WebSocket text frames** carrying a single JSON object with `type` and (for request/response pairs) `id`. All bulk frames are **WebSocket binary frames** with a fixed 16-byte header (§4).

### 2.1 Client → server

| `type` | Purpose | Carries |
|---|---|---|
| `hello` | Initial handshake | version, client_version, os, caps |
| `tool_result` | Result of a `tool_call` | id (echoes call), content, is_error, code? |
| `register_mcp` | Advertise client-side MCP capabilities | mcp_servers[] (each with tools/resources/prompts arrays) |
| `transfer_begin` | Open a file-transfer slot (client → server direction) | id, src_path, dst_device, dst_path, total_bytes, sha256, mime? |
| `transfer_progress` | Optional progress update | id, bytes_sent |
| `transfer_end` | Close a transfer slot | id, ok, error?, sha256? |
| `pong` | Heartbeat reply | id (echoes ping) |
| `ping` | Liveness probe | id |
| `error` | Out-of-band error report | id?, code, message |

### 2.2 Server → client

| `type` | Purpose | Carries |
|---|---|---|
| `hello_ack` | Handshake response | id (echoes hello), device_name, user_id, config |
| `tool_call` | Dispatch a tool to the device | id, name, args |
| `config_update` | Push a device-config change (ADR-050) | new config object |
| `transfer_begin` | Open a file-transfer slot (server → client direction) | same fields as client→server |
| `transfer_progress` | Optional progress update | id, bytes_sent |
| `transfer_end` | Close a transfer slot | same fields |
| `ping` | Liveness probe | id |
| `pong` | Heartbeat reply | id (echoes ping) |
| `error` | Out-of-band error report | id?, code, message |

The `error` frame is for protocol-level issues (malformed JSON, unknown frame type) that are not tied to a specific tool call. Tool failures travel as `tool_result` with `is_error: true` per ADR-031.

---

## 3. Tool dispatch

### 3.1 `tool_call`

Server → client. Fired when the agent loop dispatches a tool whose `plexus_device` resolves to this client.

```jsonc
{
  "type": "tool_call",
  "id": "0190d5a8-...",          // UUID v7
  "name": "exec",                // shared, client-only, or MCP-wrapped
  "args": {
    "command": "git status",
    "working_dir": "/home/alice/.plexus/",
    "timeout": 60
  }
}
```

The client validates that `name` is something it implements (file tools, exec, web_fetch, or any registered MCP entry — tool, resource wrapper, or prompt wrapper), spawns the call, and replies with `tool_result` when complete.

### 3.2 `tool_result`

Client → server.

```jsonc
{
  "type": "tool_result",
  "id": "<echoes tool_call.id>",
  "content": "On branch main\nnothing to commit, working tree clean\n",
  "is_error": false
}
```

On failure, `is_error: true` and `content` is the error message. An optional `code` field carries a stable error enum (`exec_timeout`, `sandbox_failure`, `cwd_outside_workspace`, etc. — see TOOLS.md error catalog).

The server wraps `content` with `[untrusted tool result]: ` per ADR-095 before emitting the `tool_result` content block to the LLM. The wire-level `content` here is **raw** — the client does not pre-wrap.

### 3.3 Parallel dispatch

The server may issue multiple `tool_call` frames before any `tool_result` arrives. The client spawns a tokio task per call and replies as each finishes — order is not guaranteed. Correlation is purely by `id`.

There is no per-device concurrency cap in v1. Practical bound is the agent's own parallel-tool-call output.

### 3.4 Failure paths

- **Client-side timeout** (the tool's own timeout fires): `tool_result(is_error=true, code=exec_timeout)` (or whichever tool-specific code).
- **Sandbox setup failure** (bwrap not installed, jail mount failed): `tool_result(is_error=true, code=sandbox_failure)`.
- **Disconnect mid-call** (WS closed before `tool_result` arrives): server synthesizes `tool_result(is_error=true, code=device_unreachable, content="Device <name> disconnected before completing tool call")` and feeds it into the agent loop. **No server-side retry.** Per ADR-031, the agent observes the failure and decides next action; ADR-036 trap-detection bounds runaway retries.
- **Heartbeat timeout** (2 missed pongs): same as disconnect mid-call — all in-flight calls fail with `device_unreachable`.

### 3.5 `register_mcp`

Client → server. Sent immediately after `hello_ack` if the client has any configured MCP servers, and again whenever the MCP set changes. Carries all three capability surfaces (tools, resources, prompts) for every MCP — wrapping/naming per ADR-048.

```jsonc
{
  "type": "register_mcp",
  "id": "...",
  "mcp_servers": [
    {
      "server_name": "minimax",
      "tools": [
        { "name": "web_search",    "input_schema": { ... } },
        { "name": "video_generate","input_schema": { ... } }
      ],
      "resources": [
        // Static URI:
        { "name": "index", "uri": "minimax://workspace/index" },
        // URI template (ADR-099 — placeholders surfaced as schema properties):
        { "name": "page",  "uri": "minimax://page/{page_id}" }
      ],
      "prompts": [
        { "name": "code_review",
          "arguments": [
            { "name": "language", "required": true },
            { "name": "style",    "required": false }
          ]
        }
      ]
    }
  ]
}
```

The client sends raw MCP shapes — `uri` for resources, `arguments` for prompts. The server-side registrar runs the wrap step (ADR-048): name rewriting, URI template parsing for resources, schema generation for prompts, then validation against the existing install set.

Server validates against ADR-049:
- **Within-server dup** (e.g. two tools named `search` from one MCP) → `error{code:"mcp_within_server_collision"}` and the entire registration for that server is rejected.
- **Cross-install schema drift** (e.g. `mcp_minimax_web_search` already exists with a different `input_schema` from another install site) → `error{code:"mcp_schema_collision"}` and the client's MCP set for that server is rejected (the client retains the MCP locally; the agent does not see it).

On success, the server caches the wrapped schemas, invalidates the user's tool-registry cache, and the next agent turn sees the new entries merged in alongside any server-side or other-device MCPs.

### 3.6 `config_update`

Server → client. Pushed when a `PATCH /api/devices/{name}/config` succeeds (ADR-050).

```jsonc
{
  "type": "config_update",
  "id": "...",
  "config": {
    "fs_policy": "unrestricted",
    "shell_timeout_max": 600,
    "ssrf_whitelist": ["10.180.20.30:8080", "172.20.0.5"],
    "mcp_servers": { ... },
    "workspace_path": "/home/alice/.plexus/"
  }
}
```

Client hot-reloads. In-flight tool calls finish under the **old** config (already-spawned bwrap jails are not re-mounted); new calls use the new config. Client does not ack — the next `tool_call` implicitly confirms the new config is in effect.

---

## 4. File transfer (Option A — binary frames)

### 4.1 Slot lifecycle

A transfer is a control-frame sandwich around binary data:

1. **Sender → receiver:** `transfer_begin` (text/JSON) — declares the slot.
2. **Sender → receiver:** N binary frames carrying chunks.
3. **Sender → receiver:** `transfer_end` (text/JSON) — closes the slot, asserts completion.

`id` (UUID v7) is the slot identifier. Multiple transfers may be in flight on the same WS — chunks carry the slot id in their binary header (§4.3), so they can interleave freely.

### 4.2 `transfer_begin` / `transfer_progress` / `transfer_end`

```jsonc
{
  "type": "transfer_begin",
  "id": "0190d5a9-...",            // slot id
  "direction": "client_to_server", // or "server_to_client"
  "src_device": "alice-laptop",
  "src_path": "/home/alice/.plexus/.attachments/photo.jpg",
  "dst_device": "server",
  "dst_path": "/alice-uuid/.attachments/photo.jpg",
  "total_bytes": 2_457_600,
  "sha256": "5e884898da280471...",
  "mime": "image/jpeg"             // optional, for receiver-side hinting
}

{
  "type": "transfer_progress",     // optional, for big-file UX
  "id": "0190d5a9-...",
  "bytes_sent": 1_048_576
}

{
  "type": "transfer_end",
  "id": "0190d5a9-...",
  "ok": true                       // or { "ok": false, "error": "sha256 mismatch" }
}
```

### 4.3 Binary frame layout

WebSocket binary frame, payload bytes:

```
| 16 bytes | UUID v7 — slot id (matches transfer_begin.id) |
| N bytes  | chunk bytes                                    |
```

Recommended chunk size: ~64 KB. Larger is fine; smaller adds per-frame overhead. The receiver buffers in memory only the in-flight chunk, streaming straight to disk (or to the next hop for bridge transfers).

### 4.4 Verification

Sender computes sha256 incrementally over the bytes it ships and includes the final hex digest in `transfer_begin`. Receiver computes the same sha256 as it writes, compares on `transfer_end`. Mismatch → receiver replies with `transfer_end(ok=false, error="sha256_mismatch")` and discards the partial file (deletes `dst_path` if it was created).

If the receiver runs out of disk mid-transfer, it sends `transfer_end(ok=false, error="enospc")` immediately and stops accepting binary frames for that slot.

### 4.5 Device → device

`file_transfer` between two clients (e.g. `alice-laptop` → `alice-phone`) routes through the server as a **pure bridge**:

```
sender (alice-laptop)              server                       receiver (alice-phone)
│                                  │                                  │
│── transfer_begin{id=X, ...} ──→  │                                  │
│                                  │── transfer_begin{id=X, ...} ──→  │
│── binary[id=X, chunk 0] ─────→   │── binary[id=X, chunk 0] ────→    │
│── binary[id=X, chunk 1] ─────→   │── binary[id=X, chunk 1] ────→    │
│       ...                        │        ...                       │
│── transfer_end{id=X, ok} ────→   │── transfer_end{id=X, ok} ───→    │
                                                                    [ack flows back]
│                                  │  ←── transfer_end{id=X, ok} ──── │
│  ←── transfer_end{id=X, ok} ──── │                                  │
```

The server does not buffer the full file. Each binary chunk is forwarded as it arrives (with the same slot id, which both ends agreed on). If the receiver cannot keep up, WS-level flow control naturally backpressures the sender.

If either leg disconnects mid-transfer, the server cancels the other leg with `transfer_end(ok=false, error="peer_disconnected")` and the agent observes a `tool_result(is_error=true, code=device_unreachable)`.

### 4.6 Caller-facing semantics

The agent's `file_transfer` tool blocks until the slot closes (`transfer_end` arrives, in either direction). The tool returns success when `ok=true`, or surfaces the error per ADR-031 when `ok=false`.

The `message` tool with `files: [...]` and a `plexus_device` other than `"server"` triggers an internal `transfer_begin` (client → server) before posting to the channel — agent doesn't see this transfer. If the transfer fails, the `message` tool fails.

---

## 5. Errors

### 5.1 `error` frame

For protocol-level issues only — not for tool failures (those are `tool_result` with `is_error:true`).

```jsonc
{
  "type": "error",
  "id": "<related frame id, if applicable>",
  "code": "malformed_frame" | "unknown_type" | "version_mismatch" |
          "mcp_schema_collision" | "mcp_within_server_collision" |
          "transfer_unknown_id" | ...,
  "message": "human-readable detail"
}
```

Either side may emit. Receiving an `error` does not require reconnecting unless the `code` says so (e.g. `version_mismatch`).

### 5.2 Close codes

Standard WS close codes 1000–1015, plus Plexus-specific:

| Code | Meaning |
|---|---|
| `1000` | Normal close (e.g. client shutdown). |
| `1001` | Going away (server restart). Client should reconnect. |
| `4401` | Authentication failed (token invalid / revoked). Do NOT retry. |
| `4408` | Heartbeat timeout. Client should reconnect after backoff. |
| `4409` | Protocol version unsupported. Reconnect with newer client. |

---

## 6. Versioning

Protocol version is a single string in `hello.version`. v1 is the version specified in this doc. Future breaking changes bump the major number; the server may accept multiple versions during a transition window. There is no minor/patch versioning at the protocol level — additive changes (new frame types, new fields) don't bump the version, but recipients MUST ignore unknown fields (forward compat).

---

## 7. Out of scope (M0–M3)

- **MessagePack / CBOR** — JSON for now. Revisit if frame size becomes meaningful.
- **Streaming `tool_result`** — results are single-frame even if large (subject to the tool's own result cap). Real streaming would require a slot model like transfers; not justified yet.
- **Multi-server failover** — single server per device. Multi-server coordination is ruled out (ADR-061).
- **Resume / range support for transfers** — failed transfers restart from byte 0. Resumable transfers require tracking offsets persistently; not worth the complexity at current file sizes.

---

## 8. Related ADRs

- **ADR-031** — tool failure → `tool_result(is_error:true)`.
- **ADR-047** — shared MCP client; three surfaces (tools/resources/prompts).
- **ADR-048** — MCP wrapping + naming convention; prompt-output stringify rule.
- **ADR-049** — MCP collision rejection (within-server dup + cross-install schema drift).
- **ADR-050** — device config push.
- **ADR-052** — `web_fetch` as shared tool with per-device whitelist.
- **ADR-091** — device token as PK.
- **ADR-095** — untrusted-tool-result wrap.
- **ADR-096** — this protocol's headline decisions.
- **ADR-097** — device pairing flow + token lifecycle.
- **ADR-099** — MCP resource URI templates surfaced as schema properties.
- **ADR-100** — MCP `enabled` filter applies uniformly across the three surfaces.
