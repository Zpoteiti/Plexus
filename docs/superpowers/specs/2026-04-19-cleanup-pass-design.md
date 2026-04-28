# Cleanup-Pass Design — Post-M2, Pre-Main-Merge

**Date:** 2026-04-19
**Branch:** `M3-gateway-frontend`
**Status:** design (pending plan + execution)
**Supersedes:** the three cleanup proposals in `cleanup_proposal.md` (me, Gemini, Codex) — this spec is the synthesis + new decisions layered on top.

---

## 1. Scope & Posture

- **Users: 0.** No production deployments. Dev databases are disposable.
- **Backward compatibility: not required.** Every deletion below is a hard delete, not a 410 tombstone.
- **DB schema is ours to rewrite.** No migration framework until a first real user exists.
- **Principle 1 — Generic over specialty.** If workspace + generic file ops can do the job, kill the specialty API/tool. This drives the death of `soul`, `memory_text`, `skills_api`, `save_memory`, `edit_memory`, `read_skill`, `install_skill`.
- **Principle 2 — Unified tool surface.** One schema per operation. `device_name` dispatches where it runs. No duplicate server/client tool schemas.
- **Principle 3 — Workspace is the single source of truth for user files.** No parallel `/api/files` cache. Durable files in workspace; ephemerals (chat-drop images) land in `workspace/.attachments/` with a TTL sweep.
- **Principle 4 — First-class or delete.** Stored fields without a write path violate the invariant. Either they get editors or they go.
- **Principle 5 — Drop old code, don't tombstone it.** 410 handlers, dead functions, unused variants all leave the tree.

### 1.1 Deliberately out of scope

These are defensible future passes; forcing them into this cleanup would double scope and muddy review:

- God-file splits (`api.rs` 840L, `context.rs` 992L, `agent_loop.rs` 887L, `server_tools/file_ops.rs` 1307L). Splitting before behavior is unified just spreads the mess (Codex's framing, adopted).
- Real migration framework (`sqlx::migrate!`). No data to preserve — add the day a first real user lands.
- Channel adapter trait abstraction. Per-adapter files stay honest about library coupling.
- Secret-at-rest encryption for `discord_configs.bot_token` / `telegram_configs.bot_token`. Wants its own design.
- `TestAppStateBuilder` pattern for the 4 test-only state helpers. Test-only ergonomics, no runtime value.
- Rename of `plexus-server/src/memory.rs` (context-compression module — name is misleading post-cleanup but functional).

---

## 2. Unified File-Storage Architecture

Today there are two storage systems:

- **`/api/files` + `file_store.rs`** — POST returns `file_id`, GET serves by id, bytes live on disk under a server-local path, TTL ~24h.
- **`/api/workspace/files/<path>`** — durable per-user tree, GET/PUT/DELETE by path.

**After cleanup:** one system. Workspace path is the only identifier. `file_store.rs` and `/api/files` are deleted.

### 2.1 Flow changes

**Inbound — user drags an image into chat:**

Old:
```
POST /api/files (multipart)
  → returns {file_id}
  → message carries /api/files/{id} URL
  → context.rs strips prefix, file_store::load_file() for model input
```

New:
```
PUT /api/workspace/files/.attachments/{msg_id}/{filename}
  body: raw bytes
  Content-Type: <detected mime>
  → returns {path, size_bytes, mime}

Frontend then:
  - Reads local File → base64 encode
  - Sends chat message (via WS) with content blocks:
      [
        { "type": "text", "text": "<user text>" },
        { "type": "image",
          "source": { "type": "base64", "media_type": "image/png", "data": "..." },
          "workspace_path": ".attachments/{msg_id}/{filename}"   // non-standard, ours
        }
      ]

Server stores the full content blocks in messages.content JSONB.
Context builder feeds base64 to the model (strips workspace_path before sending upstream).
Frontend re-render: uses workspace_path if the file still exists; falls back to base64 after TTL sweep.
```

The double storage is intentional: the workspace file is user-manageable (rename, move out of `.attachments/`, reuse in later messages); the embedded base64 makes conversation history durable past the TTL.

**Outbound — agent sends a workspace file to Discord:**

Old:
```
agent's message tool reads workspace file
  → file_store::save_upload() to /api/files/{id}
  → Discord adapter posts URL to Discord API
  → temp file lingers 24h
```

New:
```
agent's message tool passes {device_name:"server", path:"plan.pdf"} to adapter
  → Discord adapter opens workspace_fs::read_stream(user_id, path)
  → streams directly to Discord multipart upload
  → no temp file ever exists
```

**Outbound — agent sends a device-origin file (e.g. from `linux-devbox`):**

```
message tool receives attachments=[{device_name:"linux-devbox", path:"/home/zou/video.mp4"}]
  → open WS ReadStream request to linux-devbox
  → pipe bytes: client → server (memory buffer, not disk) → Discord multipart
  → on mid-stream failure: close, log, retry (up to 3× with exponential backoff)
  → all retries exhausted: return ToolResult::Err to agent
```

No disk touch on server. Client keeps the source file. Retry = re-request over WS.

### 2.2 `workspace/.attachments/` lifecycle

Reserved directory inside each user's workspace.

- Path: `workspace/.attachments/<YYYY-MM-DD>/<msg_id>-<filename>`
- **Counts against quota** (user decided).
- **TTL sweep:** system cron deletes files older than 30 days.
- User can move files out of `.attachments/` to keep them permanently — becomes a first-class workspace file at the new path.
- Frontend hides `.attachments/` in the default tree view (shows "Attachments (N)" as collapsible).

### 2.3 What gets deleted

Files removed entirely:

- `plexus-server/src/file_store.rs`
- `POST /api/files` handler + `GET /api/files/{id}` handler in `api.rs`
- Route registrations for both
- `plexus-frontend/src/api/upload.ts` → replaced with a thin wrapper over `POST /api/workspace/files/.attachments/...`
- `file_store`-related imports across `message.rs`, `context.rs`, `channels/*`

Replaced conceptually:

- `file_id` string type → workspace path `String` (same Rust type, different semantics)
- `ResolvedMedia` enum → just bytes + mime
- Channel adapter "resolve URL" step → "read workspace path"

---

## 3. `workspace_fs` Service Module

Today workspace writes happen in 3 sites (4 once `file_store.rs` is gone, minus that = 3 survivors), each re-implementing path resolution + quota + write + rollback:

- `plexus-server/src/api.rs:407-450` — REST workspace PUT
- `plexus-server/src/server_tools/file_ops.rs:61-123` — `write_file` server tool
- `plexus-server/src/server_tools/file_transfer.rs:38+` — `file_transfer` server tool

These collapse into one service.

### 3.1 Module layout

```
plexus-server/src/workspace/
├── mod.rs          # pub use; re-exports
├── paths.rs        # kept — path validation (internal helper)
├── quota.rs        # kept — QuotaCache (unchanged externally)
└── fs.rs           # NEW — the service; single path for read/write/delete/stream
```

### 3.2 `fs.rs` public API

```rust
pub struct WorkspaceFs {
    root: PathBuf,                 // server's PLEXUS_WORKSPACE_ROOT
    quota: Arc<QuotaCache>,
    skills_cache: Arc<SkillsCache>,
}

// Read
pub async fn read(user_id: &str, path: &str) -> Result<Vec<u8>, WorkspaceError>;
pub async fn read_stream(user_id: &str, path: &str) -> Result<ReaderStream<tokio::fs::File>, WorkspaceError>;
pub async fn stat(user_id: &str, path: &str) -> Result<FileStat, WorkspaceError>;

// Write (owns quota reserve + rollback + skills invalidation)
pub async fn write(user_id: &str, path: &str, bytes: &[u8]) -> Result<(), WorkspaceError>;
pub async fn write_stream<R: AsyncRead + Unpin>(
    user_id: &str, path: &str, reader: R, expected_size: u64,
) -> Result<(), WorkspaceError>;

// Delete
pub async fn delete(user_id: &str, path: &str) -> Result<(), WorkspaceError>;
pub async fn delete_prefix(user_id: &str, prefix: &str) -> Result<u64, WorkspaceError>;  // TTL sweep

// List / search
pub async fn list(user_id: &str, path: &str) -> Result<Vec<DirEntry>, WorkspaceError>;
pub async fn glob(user_id: &str, pattern: &str, root: &str) -> Result<Vec<String>, WorkspaceError>;
pub async fn grep(user_id: &str, pattern: &str, root: &str, opts: GrepOpts) -> Result<Vec<GrepHit>, WorkspaceError>;

// Quota
pub fn quota(user_id: &str) -> QuotaSnapshot;
pub async fn wipe_user(user_id: &str) -> Result<(), WorkspaceError>;  // account deletion
```

### 3.3 Responsibilities owned by `fs.rs` (not its callers)

Every public function handles the full stack:

1. **Path resolution** — internal, no caller-facing `resolve_user_path_*`.
2. **Permission / symlink-escape check** — resolved path must stay inside user's root; violations logged at `warn` level.
3. **Quota reservation + rollback** — `write`/`write_stream` call `quota.check_and_reserve_upload()` before write, `forget` on failure.
4. **Skills cache invalidation** — any write under `skills/<skill_name>/` triggers `skills_cache.invalidate(user_id)`.
5. **MIME detection on read** — returned via `FileStat.mime` using the unified `plexus_common::mime` helper.
6. **Error typing** — all errors are `WorkspaceError` variants from `plexus-common::errors::workspace`.

### 3.4 Path policy

- **Agent tool calls (both server-side dispatch and client-side adapter) require an absolute path.** Enforced at the tool-handler layer before `workspace_fs` is reached. Absolute paths must have the target's canonical root as prefix.
- **REST handlers accept relative paths** — `axum`'s `Path` capture strips the leading slash; per-user auth supplies `user_id`. No enforcement beyond prefix-check after resolution.

### 3.5 `ERROR:{filename}` sentinel removed

`WorkspaceUploadResult` changes shape:

```rust
// Before
pub struct WorkspaceUploadResult {
    pub path: String,           // or "ERROR:{filename}" on failure
    pub size_bytes: u64,
}

// After
pub struct WorkspaceUploadResult {
    pub filename: String,
    pub outcome: Result<Uploaded, UploadError>,  // serde-tagged
}
pub struct Uploaded { pub path: String, pub size_bytes: u64 }
pub enum UploadError { Quota { remaining: u64 }, TooLarge, Io(String) }
```

Frontend `Workspace.tsx` pattern-matches on `outcome`.

### 3.6 Thin-wrapper callers

- `api.rs` handlers — 5-15 line functions that extract auth, call `workspace_fs::*`, map errors to HTTP status. No business logic.
- `server_tools/file_ops.rs` — each tool (read_file, write_file, edit_file, list_dir, glob, grep) drops its own FS logic; for `device_name="server"` it calls `workspace_fs::*`, otherwise forwards to the WS routing layer (Section 4).
- `server_tools/file_transfer.rs` — one side of every transfer is now `workspace_fs::read_stream` or `write_stream`; the other side stays on client-WS.

### 3.7 Tests

- Unit tests hit the real filesystem via `tempfile::tempdir` (no mocks).
- Existing integration tests keep passing; they go through the new module transparently.
- New test: symlink-escape is rejected AND logged.
- New test: `.attachments/` files count against quota.

---

## 4. Unified Tool Contract

### 4.1 Direction

**Before:** server registers 8 server tools, client registers 7 client tools. Server and client BOTH have `edit_file`/`read_file`/etc. with different arg names (`path` vs `file_path`) and different matching semantics. Agent sees both sets as distinct tools keyed off the hosting layer.

**After:** one schema per operation. `device_name` arg picks the target. Server-only tools (physically only make sense on server) stay separate, omitting `device_name`. MCP tools use `mcp_<mcp_server>_<tool>` naming with a `device_name` enum.

### 4.2 Final tool set

**File tools (unified, nanobot-shaped, device_name-routed) — 6 tools:**

```
read_file    {device_name, path, offset?, limit?}
write_file   {device_name, path, content}
edit_file    {device_name, path, old_text, new_text, replace_all?}
list_dir     {device_name, path}
glob         {device_name, pattern, path?}
grep         {device_name, pattern, path?, type?, glob?, context?, case_insensitive?}
```

**Shell tool — 1 (device_name routed, `server` rejected):**

```
shell        {device_name (non-"server"), command, working_dir?, timeout?}
```

**Server-only tools — 4:**

```
message        {text, attachments[]?}         // attachments carry {device_name, path}
web_fetch      {url, method?, headers?, body?}  // SSRF-blocked, no whitelist ever
cron           {name, schedule, prompt}
file_transfer  {from_device, from_path, to_device, to_path}  // streaming, retry
```

**MCP tools — N**, each wrapped as `mcp_<mcp_server>_<tool>` with a `device_name` enum.

**Total:** 6 file + 1 shell + 4 server + N MCP. Gone: `save_memory`, `edit_memory`, `read_skill`, `install_skill` (all 4 replaced by edit_file/read_file/write_file against server workspace + auto-invalidation on `skills/` writes).

### 4.3 `edit_file` canonical schema (nanobot-derived + device_name)

```jsonc
{
  "name": "edit_file",
  "description": "Edit a file by replacing old_text with new_text. Tolerates minor whitespace/indentation differences and curly/straight quote mismatches. If old_text matches multiple times, provide more context or set replace_all=true.",
  "input_schema": {
    "type": "object",
    "properties": {
      "device_name": {
        "type": "string",
        "enum": ["server", "linux-devbox", "mac-mini"],
        "description": "Where the file lives. 'server' for managed workspace; otherwise a connected device."
      },
      "path":        { "type": "string",  "description": "Absolute path. Must be inside the target's workspace root." },
      "old_text":    { "type": "string",  "description": "The text to find and replace." },
      "new_text":    { "type": "string",  "description": "The text to replace with." },
      "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)." }
    },
    "required": ["device_name", "path", "old_text", "new_text"],
    "additionalProperties": false
  }
}
```

**Matching semantics (identical on server and client, ported from nanobot):**

1. Exact substring match.
2. Line-trimmed sliding window (indentation drift).
3. Smart-quote normalization (curly ↔ straight).
4. Multi-match → error demanding more context, unless `replace_all=true`.
5. Create-file shortcut: `old_text=""` + file doesn't exist → create with `new_text`.

### 4.4 Routing mechanism

```rust
// plexus-server/src/server_tools/dispatch.rs (NEW)
pub async fn dispatch_file_tool(
    state: &AppState,
    user_id: &str,
    tool: FileTool,       // typed enum: EditFile, ReadFile, etc.
) -> Result<ToolResult, ToolError> {
    match tool.device_name() {
        "server" => run_on_server_workspace(state, user_id, tool).await,   // workspace_fs
        other    => forward_to_client(state, user_id, other, tool).await,  // WS round-trip
    }
}
```

- `run_on_server_workspace` validates path (absolute; inside `$WORKSPACE_ROOT/<user_id>/`) then calls `workspace_fs::*`.
- `forward_to_client` sends `ServerToClient::ToolCall { tool, args, device_name }` over WS; client runs its local implementation; server awaits `ClientToServer::ToolResult`. Timeout = `shell_timeout_max` for shell, 60s default for file ops.

### 4.5 Client-side unification

- Client modules (`plexus-client/src/tools/{edit,read,write,list_dir,glob,grep,shell}.rs`) keep their `execute(args)` implementations.
- Client **drops** `input_schema()` / `description()` — server owns the schema now.
- `edit_file::execute` uses the shared matcher from `plexus-common::fuzzy_match`.
- Client registers tool NAMES only (for capability negotiation); schemas live server-side.

### 4.6 MCP tool naming + collision handling

Wrapping rule: every MCP tool becomes `mcp_<mcp_server_name>_<tool_name>` with `device_name` enum listing all install sites.

**Collision rule:** same `mcp_<server>_<tool>` name MUST have identical tool schemas across install sites. On install (`PUT /api/devices/{name}/mcp` or `PUT /api/server-mcp`), the incoming MCP's tool list is fetched and each tool schema compared against existing installs of the same `<server>` name. Any mismatch returns **409 Conflict** with a structured diff body:

```jsonc
{
  "error": "mcp_schema_conflict",
  "message": "An MCP server named 'MINIMAX' is already installed with a different tool schema. Rename your install or ask admin to upgrade the shared one.",
  "conflicts": [
    {
      "tool": "web_search",
      "existing_schema": { "properties": ["query"] },
      "your_schema":     { "properties": ["query", "search_engine"] },
      "installed_on":    ["server"]
    }
  ]
}
```

Resolution: user renames their install (e.g. `MINIMAX` → `MINIMAX_V2`) or admin upgrades the shared install. No auto-suffix; no silent merging.

### 4.7 Wrapping logic

`plexus-server/src/mcp/wrap.rs` (NEW, ~60 lines) takes the raw MCP tool schema, injects `device_name` into `properties` + `required`, prefixes name with `mcp_<server>_`. Called once at tool-list-build time per session.

### 4.8 Net code delta

- Deleted: `save_memory.rs`, `edit_memory.rs`, `read_skill.rs`, `install_skill.rs` (~400 lines combined); client-side `input_schema()` in each file tool.
- Added: `server_tools/dispatch.rs` (~80 lines), `mcp/wrap.rs` (~60 lines), shared edit-match call sites.
- Net reduction.

---

## 5. Server-Only Tool Details

Four tools. Covering only the two that change meaningfully in this cleanup. `cron` and `web_fetch` get confirmation at the bottom.

### 5.1 `message` tool

Contract:

```jsonc
{
  "name": "message",
  "description": "Send a message to the user through their current channel (Discord, Telegram, or browser). Attachments stream from their source; no staging.",
  "input_schema": {
    "type": "object",
    "properties": {
      "text": { "type": "string", "description": "Message body. Markdown on Discord; plain on Telegram." },
      "attachments": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "device_name": { "type": "string", "enum": ["server", "linux-devbox", "mac-mini"] },
            "path":        { "type": "string", "description": "Absolute path on the source." }
          },
          "required": ["device_name", "path"]
        }
      }
    },
    "required": ["text"]
  }
}
```

Delivery logic (pseudocode):

```rust
async fn message_tool(state, user_id, text, attachments) -> ToolResult {
    let channel = current_channel(user_id)?;  // Discord | Telegram | Gateway

    let streams = attachments.into_iter().map(|att| {
        match att.device_name.as_str() {
            "server" => workspace_fs::read_stream(user_id, &att.path),
            device   => open_device_stream(state, user_id, device, &att.path),
        }
    });

    let mut retries = 0;
    loop {
        match channel.send(&text, streams.clone()).await {
            Ok(()) => return Ok(ToolResult::Ok),
            Err(e) if e.is_retriable() && retries < 3 => {
                retries += 1;
                tracing::warn!("message send retry {}/3: {}", retries, e);
                tokio::time::sleep(Duration::from_millis(500 * 2_u64.pow(retries))).await;
            }
            Err(e) => return Ok(ToolResult::Err(e.to_string())),
        }
    }
}
```

Stream adapters per channel:
- **Discord** (serenity): `send_files` accepts `AttachmentType` wrapping a `Read`. Direct.
- **Telegram** (teloxide): `SendDocument` + `InputFile::memory` / `InputFile::url`. Buffered stream adapter.
- **Gateway** (browser): `OutboundFrame::Message` carries workspace path (server-origin) or `/api/device-stream/<device>/<path>` URL (device-origin); browser fetches at render time.

### 5.2 `file_transfer` tool

Contract (generalizes to device↔device):

```jsonc
{
  "name": "file_transfer",
  "description": "Stream a file between two targets. Efficient for large files; bypasses read_file/write_file base64 overhead.",
  "input_schema": {
    "type": "object",
    "properties": {
      "from_device": { "type": "string", "enum": ["server", "linux-devbox", "mac-mini"] },
      "from_path":   { "type": "string", "description": "Absolute path on source." },
      "to_device":   { "type": "string", "enum": ["server", "linux-devbox", "mac-mini"] },
      "to_path":     { "type": "string", "description": "Absolute path on destination." }
    },
    "required": ["from_device", "from_path", "to_device", "to_path"]
  }
}
```

Dispatch matrix:

| from_device | to_device | Mechanism |
|---|---|---|
| `server` | `server` | `workspace_fs::copy` (local; both paths validated against user root) |
| `server` | device | `workspace_fs::read_stream` → push chunks over WS to device's `write_stream` handler |
| device | `server` | Pull chunks from source device over WS → `workspace_fs::write_stream` on server |
| device_a | device_b | Pull A → relay through server (no disk) → push B |

Retry: up to 3 with exponential backoff; non-retriable errors (quota, path-outside-root, permission) fail immediately.

### 5.3 `web_fetch`

Server-only, runs in prod — **must unconditionally block RFC-1918, link-local, loopback, carrier-grade NAT.** No per-user whitelist, no per-device whitelist, no exception ever:

```jsonc
{
  "name": "web_fetch",
  "description": "Fetch a public URL from the server. Cannot reach private networks; use client-side tools on a device with ssrf_whitelist configured for intranet access.",
  "input_schema": {
    "properties": {
      "url":    { "type": "string" },
      "method": { "type": "string", "enum": ["GET", "POST"], "default": "GET" },
      "headers":{ "type": "object" },
      "body":   { "type": "string" }
    },
    "required": ["url"]
  }
}
```

Implementation imports `plexus-common::network::validate_url` with an empty whitelist.

### 5.4 `cron`

No changes.

### 5.5 Device-stream endpoint for browser

```
GET /api/device-stream/{device_name}/{path:.*}
```

- Auth: JWT → `user_id`.
- Validates device belongs to user.
- Opens WS `ReadStream` to the device, pipes bytes through the HTTP response.
- No disk touch. No cache. Each hit is a fresh WS stream.

---

## 6. Device Config Fields as First-Class

Three fields go from stored-but-unwritable to fully editable: `workspace_path`, `shell_timeout_max`, `ssrf_whitelist`. Plus this formalizes the existing editable `fs_policy` and per-device `mcp_servers`.

### 6.1 Canonical `devices` table columns

(Full table definition in Section 8.2.)

| Column | Type | Purpose |
|---|---|---|
| `workspace_path` | `TEXT NOT NULL` | Absolute path; bwrap jail root. Agent's "target workspace root" for this device. Default at create: `~/.plexus/workspace/<name>`. |
| `shell_timeout_max` | `INTEGER NOT NULL DEFAULT 300` | Cap in seconds. Agent self-selects per-call via `timeout` arg, capped at this value (hard ceiling 1800 enforced in code, ported from nanobot `_MAX_TIMEOUT`). |
| `ssrf_whitelist` | `TEXT[] NOT NULL DEFAULT '{}'` | CIDRs that bypass default RFC-1918 block on THIS device's client-side ops. Each validated as `IpNetwork` at write time. |
| `fs_policy` | `TEXT NOT NULL DEFAULT 'sandbox'` | `'sandbox'` (bwrap) or `'unrestricted'` (full client disk). `unrestricted` requires typed-confirmation modal in UI. |
| `mcp_servers` | `JSONB NOT NULL DEFAULT '[]'` | Array of MCP config objects `{name, transport, url, env}`. |

### 6.2 REST endpoints

(Full table in Section 7.)

- `POST /api/devices` — create device + initial config (requires `workspace_path`).
- `GET /api/devices` — list user's devices with full config.
- `GET /api/devices/{name}/config` — full config for one device. (Renamed from `/policy`.)
- `PATCH /api/devices/{name}/config` — partial update: workspace_path, shell_timeout_max, ssrf_whitelist, fs_policy. Per-field validation; 422 on invalid CIDR.
- `DELETE /api/devices/{name}` — revoke + remove.
- `GET /api/devices/{name}/mcp` — read full MCP list.
- `PUT /api/devices/{name}/mcp` — replace full list. Schema-collision check fires per-item during validate phase; 409 on any collision; atomic.

### 6.3 Push-based config propagation

When `PATCH /api/devices/{name}/config` commits:

1. Update DB row.
2. Look up device's current WS connection.
3. Send `ServerToClient::ConfigUpdate { workspace_path, shell_timeout_max, ssrf_whitelist, fs_policy, mcp_servers }`.
4. Client replaces in-memory config. If `workspace_path` changed, client tears down bwrap jail and reconnects (document this behavior in UI: "Changing workspace path will reconnect the device").
5. If device offline, config takes effect on next connection.

`ConfigUpdate` is an additive protocol variant in `plexus-common::protocol`.

### 6.4 System prompt inclusion

Extend the device-status block in the agent's system prompt:

```
## Your targets

### server
workspace_root: /var/lib/plexus/workspace/yucheng

### linux-devbox (last seen 2m ago)
workspace_root: /home/zou/projects
shell_timeout_max: 600s (agent may pass lower via `timeout` arg)
ssrf_whitelist: 10.180.0.0/16
fs_policy: sandbox
mcp_servers: MINIMAX, github

### mac-mini (offline)
workspace_root: /Users/zou/plexus
shell_timeout_max: 300s
ssrf_whitelist: (none; default RFC-1918 block applies)
fs_policy: sandbox
mcp_servers: (none)
```

Replaces the current ad-hoc "workspace_path echo" in `context.rs:455`.

### 6.5 Settings.tsx UI

- **Devices tab** (new or extension):
  - Device list: name, last-seen, online/offline chip.
  - "Add device" modal: `name`, `workspace_path` (suggestion: `~/.plexus/workspace/<name>`), copy-token flow.
  - Device detail page:
    - Editable: `workspace_path`, `shell_timeout_max` (10–1800), `fs_policy` (toggle with warning-modal for unrestricted).
    - `ssrf_whitelist` multi-input, each CIDR validated live.
    - MCP section: list, add, edit, remove (uses `PUT /api/devices/{name}/mcp` — replace-all).
    - "Save" → `PATCH /api/devices/{name}/config`.
    - "Revoke" → confirm modal → `DELETE /api/devices/{name}`.

- **Unrestricted fs_policy UX:** typed-confirmation modal — user types the device name to confirm. Matches the account-deletion pattern.

### 6.6 SSRF policy

- **Server `web_fetch`**: unconditional block of RFC-1918 + link-local + loopback + carrier-grade NAT. No whitelist exists.
- **Client ops (shell, subprocess, client-side MCP network calls)**: default block, per-device `ssrf_whitelist` punches holes.
- Shared CIDR logic in `plexus-common::network`:
  ```rust
  pub const BLOCKED_NETWORKS: &[IpNetwork] = &[
      ipnetwork!("0.0.0.0/8"), ipnetwork!("10.0.0.0/8"),
      ipnetwork!("100.64.0.0/10"), ipnetwork!("127.0.0.0/8"),
      ipnetwork!("169.254.0.0/16"), ipnetwork!("172.16.0.0/12"),
      ipnetwork!("192.168.0.0/16"), ipnetwork!("::1/128"),
      ipnetwork!("fc00::/7"), ipnetwork!("fe80::/10"),
  ];
  pub fn validate_url(url: &str, whitelist: &[IpNetwork]) -> Result<(), NetworkError>;
  ```

---

## 7. API Surface — Full Post-Cleanup State

**Legend:** 🟢 KEEP · 🟡 CHANGE · 🔴 DELETE · ✨ NEW

### Auth
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `POST` | `/api/auth/register` | Create a user account |
| 🟢 | `POST` | `/api/auth/login` | Exchange credentials for JWT |
| 🟢 | `DELETE` | `/api/user` | Account self-deletion (wipes workspace, DB cascade) |

### User profile
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `GET` | `/api/user/profile` | Logged-in user's profile |
| 🟢 | `PATCH` | `/api/user/display-name` | Edit display name |
| 🔴 | `GET/PATCH` | `/api/user/soul` | Soul retired |
| 🔴 | `GET/PATCH` | `/api/user/memory` | Replaced by `/api/workspace/files/MEMORY.md` |

### Sessions
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `GET` | `/api/sessions` | List user's chat sessions |
| 🟢 | `DELETE` | `/api/sessions/{session_id}` | Delete a session |
| 🟢 | `GET` | `/api/sessions/{session_id}/messages` | Message history |

### Files (OLD dual storage — ALL DIE)
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🔴 | `POST` | `/api/files` | Ephemeral upload cache |
| 🔴 | `GET` | `/api/files/{file_id}` | Ephemeral download |

### Workspace
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `GET` | `/api/workspace/quota` | Usage + limit |
| 🟡 | `GET` | `/api/workspace/tree` | Tree; each leaf gets MIME from `plexus-common::mime`; `.attachments/` collapsed by default |
| 🟡 | `GET` | `/api/workspace/files/{path:.*}` | Streamed via `ReaderStream` |
| 🟡 | `PUT` | `/api/workspace/files/{path:.*}` | Quota check + skills invalidate on `skills/` |
| 🟢 | `DELETE` | `/api/workspace/files/{path:.*}` | Delete |
| 🟡 | `POST` | `/api/workspace/upload` | Typed `{filename, outcome: Result<Uploaded, UploadError>}` |
| 🟢 | `GET` | `/api/workspace/skills` | List skills |

Chat-drop images use `PUT /api/workspace/files/.attachments/{msg_id}/{filename}` — no new endpoint needed; reuses the existing workspace PUT.

Current code uses `/api/workspace/file` (singular). Renames to `/api/workspace/files/{path:.*}` (plural + catch-all).

### Devices
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `POST/GET` | `/api/device-tokens` | Create/list pairing tokens |
| 🟢 | `DELETE` | `/api/device-tokens/{token}` | Revoke token |
| 🟡 | `GET` | `/api/devices` | List with full config |
| 🟡 | `GET/PATCH` | `/api/devices/{device_name}/config` | Renamed from `/policy`; expanded to cover `workspace_path`, `shell_timeout_max`, `ssrf_whitelist`, `fs_policy` |
| 🟢 | `GET/PUT` | `/api/devices/{device_name}/mcp` | Replace-all; schema-collision check on PUT |

### Cron
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `GET/POST` | `/api/cron-jobs` | List / create |
| 🟢 | `PATCH/DELETE` | `/api/cron-jobs/{job_id}` | Edit / delete |

### Channels
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `POST/GET/DELETE` | `/api/discord-config` | Discord bot config |
| 🟢 | `POST/GET/DELETE` | `/api/telegram-config` | Telegram bot config |

### Streaming
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| ✨ | `GET` | `/api/device-stream/{device_name}/{path:.*}` | Browser-to-device file stream (WS relay) |

### Admin
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🔴 | `GET/PUT` | `/api/admin/default-soul` | Soul retired |
| 🟢 | `GET/PUT` | `/api/admin/rate-limit` | Global rate limits |
| 🟢 | `GET/PUT` | `/api/llm-config` | LLM config |
| 🟢 | `GET/PUT` | `/api/server-mcp` | Admin-installed server-side MCPs |
| 🟢 | `GET` | `/api/admin/users` | List users |
| 🟢 | `DELETE` | `/api/admin/users/{user_id}` | Admin-initiated deletion |

### Skills API (ALL DIE)
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🔴 | `GET/POST` | `/api/skills` | Replaced by `/api/workspace/skills` |
| 🔴 | `POST` | `/api/skills/install` | Agent writes under `workspace/skills/` |
| 🔴 | `DELETE` | `/api/skills/{name}` | Agent uses workspace DELETE |

### WebSocket
| Status | Method | Path | Purpose |
|:-:|:-:|---|---|
| 🟢 | `GET` | `/ws` | Device + gateway WS endpoint |

### Delta summary

| Change | Count |
|---|---:|
| 🔴 Deleted handlers | 11 (soul×2, memory×2, files×2, skills×3, default-soul×2) |
| 🔴 Deleted modules | 2 (`skills_api.rs`, `file_store.rs`) |
| 🟡 Changed | 5 (workspace tree/get/put/upload + policy→config rename) |
| ✨ Added | 1 (device-stream GET — chat-drop reuses existing workspace PUT) |
| 🟢 Kept unchanged | 19 |

---

## 8. DB Schema

One file: `plexus-server/src/db/schema.sql`. Loaded at startup via `include_str!`. No migration framework, no cascade loop, no `ALTER TABLE` at boot.

### 8.1 What dies

- The cascade-migration loop in `plexus-server/src/db/mod.rs` (~100 lines)
- 12× `ALTER TABLE ADD COLUMN IF NOT EXISTS` → folded into `CREATE TABLE`
- 2× `DROP COLUMN IF EXISTS` → gone (those columns aren't defined)
- 1× `DROP TABLE IF EXISTS skills` → gone (skills table never created)
- Constraint mutation logic at `db/mod.rs:193-194` → folded into table definitions

Startup becomes:
```rust
pub async fn initialize(pool: &PgPool) -> Result<()> {
    sqlx::query(include_str!("schema.sql")).execute(pool).await?;
    seed_system_config(pool).await?;
    Ok(())
}
```

Columns dropped:

| Table | Column | Reason |
|---|---|---|
| `users` | `ssrf_whitelist` | Per-user whitelist dies (Section 5.3) |
| `users` | `soul` | Soul retired |
| `users` | `memory_text` | Memory lives in workspace as `MEMORY.md` |
| `devices` | `shell_timeout` | Renamed to `shell_timeout_max` |
| — | entire `skills` table | Filesystem-only under `workspace/skills/` |

### 8.2 Canonical `schema.sql`

```sql
CREATE TABLE IF NOT EXISTS users (
    id              TEXT        PRIMARY KEY,
    email           TEXT        NOT NULL UNIQUE,
    password_hash   TEXT        NOT NULL,
    display_name    TEXT        NOT NULL,
    timezone        TEXT        NOT NULL DEFAULT 'UTC',
    is_admin        BOOLEAN     NOT NULL DEFAULT FALSE,
    last_dream_at   TIMESTAMPTZ,
    last_heartbeat_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS devices (
    id                  TEXT        PRIMARY KEY,
    user_id             TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                TEXT        NOT NULL,
    token_hash          TEXT        NOT NULL,
    workspace_path      TEXT        NOT NULL,
    shell_timeout_max   INTEGER     NOT NULL DEFAULT 300,
    ssrf_whitelist      TEXT[]      NOT NULL DEFAULT '{}',
    fs_policy           TEXT        NOT NULL DEFAULT 'sandbox'
                         CHECK (fs_policy IN ('sandbox', 'unrestricted')),
    mcp_servers         JSONB       NOT NULL DEFAULT '[]',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at        TIMESTAMPTZ,
    UNIQUE (user_id, name)
);
CREATE INDEX IF NOT EXISTS idx_devices_user_id ON devices(user_id);

CREATE TABLE IF NOT EXISTS device_tokens (
    token           TEXT        PRIMARY KEY,
    user_id         TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    consumed_at     TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_device_tokens_user_id ON device_tokens(user_id);

CREATE TABLE IF NOT EXISTS sessions (
    id              TEXT        PRIMARY KEY,
    user_id         TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel         TEXT        NOT NULL,           -- "gateway" | "discord" | "tg:{chat_id}"
    title           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_activity   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, channel)
);
CREATE INDEX IF NOT EXISTS idx_sessions_user_id_last_activity ON sessions(user_id, last_activity DESC);

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT        PRIMARY KEY,
    session_id      TEXT        NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role            TEXT        NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
    content         JSONB       NOT NULL,           -- Anthropic-style content blocks
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_messages_session_id_created ON messages(session_id, created_at);

CREATE TABLE IF NOT EXISTS cron_jobs (
    id                  TEXT        PRIMARY KEY,
    user_id             TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                TEXT        NOT NULL,
    kind                TEXT        NOT NULL DEFAULT 'user'
                         CHECK (kind IN ('user', 'system')),
    schedule            TEXT        NOT NULL,
    prompt              TEXT        NOT NULL,
    enabled             BOOLEAN     NOT NULL DEFAULT TRUE,
    last_run_at         TIMESTAMPTZ,
    next_run_at         TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_user_id ON cron_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run_at) WHERE enabled = TRUE;
CREATE UNIQUE INDEX IF NOT EXISTS idx_cron_jobs_system_per_user
    ON cron_jobs(user_id, name) WHERE kind = 'system';

CREATE TABLE IF NOT EXISTS discord_configs (
    user_id         TEXT        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    bot_token       TEXT        NOT NULL,
    channel_id      TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS telegram_configs (
    user_id         TEXT        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    bot_token       TEXT        NOT NULL,
    allowed_chat_ids TEXT[]     NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS system_config (
    key             TEXT        PRIMARY KEY,
    value           JSONB       NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
-- Seeded keys: llm_config, rate_limit, dream_phase1_prompt, dream_phase2_prompt,
-- heartbeat_phase1_prompt, server_mcp, workspace_quota_default_bytes, etc.
```

Every FK has `ON DELETE CASCADE` inline. Account deletion is `DELETE FROM users WHERE id = $1`; cascade hits devices, device_tokens, sessions, messages, cron_jobs, discord_configs, telegram_configs. `account.rs::delete_user_everywhere` keeps orchestrating the WS kick + workspace wipe + skills cache invalidation around that one DB call.

### 8.3 Dev DB reset

```bash
# scripts/reset-db.sh
dropdb --if-exists plexus
createdb plexus
psql plexus -c "CREATE EXTENSION IF NOT EXISTS pgcrypto;"
# schema.sql loaded automatically on server start
```

Documented in README + DEPLOYMENT.md.

### 8.4 Test strategy

- `schema.sql` loaded into ignore-gated integration tests the same way it loads in production.
- Existing integration tests re-run unchanged.
- New smoke test: `initialize()` on a fresh DB produces the expected schema (verified via `information_schema.tables` + column count).

---

## 9. Mechanical Sweep

Everything that doesn't fit a bigger section. Each item is small, independent, largely mechanical.

### 9.1 Consolidation into `plexus-common`

| Module | Source sites collapsed | Consumers |
|---|---|---|
| `plexus-common/src/mime.rs` (expand) | existing `plexus-common::mime` + `api.rs::mime_from_path` + `context.rs::mime_from_filename` | server API, context builder, workspace_fs |
| `plexus-common/src/fuzzy_match.rs` (NEW) | nanobot-derived matcher | server `edit_file` dispatch, client `edit_file::execute` |
| `plexus-common/src/network.rs` (NEW) | port of nanobot `security/network.py` | server `web_fetch` (empty whitelist), client network ops (per-device whitelist) |
| `plexus-common/src/errors/*` (NEW tree) | `WorkspaceError`, `QuotaError` (flatten into WorkspaceError), `ToolError`, `AuthError`, `ProtocolError`, `McpError` | all crates |
| `plexus-common/src/protocol.rs` (extend) | add `ConfigUpdate`, `ReadStream`, `StreamChunk`, `StreamEnd` frames | server + client + gateway |

### 9.2 State mutability cleanup

`Arc<RwLock<String>> → Arc<str>` for three prompt fields in `plexus-server/src/state.rs`:
- `dream_phase1_prompt`
- `dream_phase2_prompt`
- `heartbeat_phase1_prompt`

Verification confirmed load-once-at-boot, never mutated. Init via `Arc::from(s)`; strip `.read().await` at ~6 callsites across `dream.rs` / `heartbeat.rs` / `context.rs`.

### 9.3 Dead function removal

`plexus-server/src/db/users.rs:64` — `update_timezone` has zero callers. Delete.

### 9.4 `WorkspaceUploadResult` restructure

Covered in 3.5.

### 9.5 Doc + env cleanup

| File | Change |
|---|---|
| `.env` | Remove `PLEXUS_SKILLS_DIR` |
| `plexus-server/README.md` | Remove `PLEXUS_SKILLS_DIR`; confirm `PLEXUS_WORKSPACE_ROOT` present |
| `plexus-server/docs/DEPLOYMENT.md` | Same |
| `plexus-server/docs/SECURITY.md` | Audit: remove soul, memory-as-endpoint, three-tier FsPolicy (it's two-tier). Reflect SSRF policy: server hardcoded block, per-device whitelist |
| Root `README.md` | Post-cleanup sentence pass. Drop `/api/files`, `/api/user/soul`, `/api/user/memory`, `/api/skills/*`. Mention unified file model |

### 9.6 ErrorCode + consts audit

Grep every `ErrorCode::<Variant>` use site; delete variants with zero callers. Same for `plexus-common/src/consts.rs`. Fires AFTER Sections 4-7 deletions.

### 9.7 Old imports + dead modules

- `use crate::file_store::*;` — grep + delete
- `mod file_store;` in `lib.rs` / `main.rs` — delete
- `mod skills_api;` in `auth/mod.rs` — delete
- `plexus-server/src/memory.rs` — **KEEP** (this is context-compression, not user-memory storage)

### 9.8 Client-side tools cleanup

`plexus-client/src/tools/`:
- Drop `input_schema()` / `description()` in each tool module (server owns schema now).
- `edit_file::execute` uses shared `plexus-common::fuzzy_match`.
- Normalize arg extraction to canonical names (`path` everywhere; `pattern` for glob/grep).
- Delete any tool registry that builds descriptions; client only registers capability names + execute fns.

### 9.9 Gateway sweep

Light. Verification confirmed all 5 `OutboundFrame` variants are consumed.
- Handle new protocol variants from 9.1 (or explicitly drop them if gateway sees browser frames only).
- No independent deletions beyond imports chained from deleted server modules.

---

## 10. Frontend Adjustments

Driven by the API changes in Section 7.

### 10.1 `Workspace.tsx`

- Upload handler: switch to new `POST /api/workspace/upload` response shape (pattern-match on `outcome`).
- File viewer: accept streamed response (`fetch` + `blob()` unchanged).
- Tree view: `.attachments/` collapsed as "Attachments (N)".
- NEW: inline-render `MEMORY.md` when selected — markdown render with "Edit" button opening workspace edit mode.
- NEW: inline-render `skills/<name>/SKILL.md` similarly.

### 10.2 `Settings.tsx`

- Soul/Memory sections: confirm already removed; no further action.
- Skills section: collapse to a "See Workspace → Skills" pointer.
- **NEW: Devices tab** per Section 6.5. Per-device edit modal. Unrestricted toggle uses typed-confirmation modal.
- Remove any per-user SSRF whitelist UI (dies with Section 5.3 change).

### 10.3 `Admin.tsx`

- Users tab: no change.
- NEW: **Server MCPs tab** — list admin-installed MCPs, add/remove. Uses existing `GET/PUT /api/server-mcp`.

### 10.4 `Chat.tsx` (or wherever composition lives)

- Image-drop handler: `PUT /api/workspace/files/.attachments/{msg_id}/{filename}` with raw body (not `/api/files`).
- Outbound message with attachment: carry workspace path AND base64 content blocks (Section 2.1).
- Display inbound attachments from agent: resolve via `/api/workspace/files/<path>` (server-origin) or `/api/device-stream/<device>/<path>` (device-origin).
- Message re-render: prefer workspace URL for images; fall back to base64 from message if workspace file 404s (post-TTL).

### 10.5 Frontend upload helper

`plexus-frontend/src/api/upload.ts` rewritten as thin wrapper over the new workspace upload endpoint. Tiny.

---

## 11. Error Types — All in `plexus-common`

```
plexus-common/src/errors/
├── mod.rs              # re-exports
├── workspace.rs        # WorkspaceError (incl. all Quota variants — flattened)
├── tool.rs             # ToolError (execution failures, timeouts, device-unreachable)
├── auth.rs             # AuthError (token invalid, expired, forbidden)
├── protocol.rs         # ProtocolError (WS frame malformed, version mismatch)
└── mcp.rs              # McpError (server unreachable, schema collision)
```

- `ErrorCode` enum (already in common) stays as wire discriminant.
- Each typed error implements `fn code(&self) -> ErrorCode`.
- HTTP mapping (`ApiError → StatusCode`) stays in `plexus-server`, wraps the common types. Server layer translates to HTTP; never defines new error types.
- `QuotaError` → flattened into `WorkspaceError::UploadTooLarge { limit, actual }`, `WorkspaceError::SoftLocked`. Drop `QuotaError` as a separate type.

One source of truth for what can go wrong anywhere in the system.

---

## 12. Test & Verification Strategy

- **Unit tests** per new module (workspace_fs, fuzzy_match, network). Use `tempfile::tempdir` for FS; no mocks.
- **Integration tests** (ignore-gated, real PostgreSQL): existing suite re-runs; fixtures unaffected by column removal.
- **New tests required:**
  - Symlink-escape rejected and logged (workspace_fs).
  - `.attachments/` writes count against quota.
  - `edit_file` matcher identical behavior on server and client (one shared test suite in plexus-common, imported by both).
  - MCP collision detection returns 409 with diff body.
  - `/api/device-stream` streams device-origin bytes end-to-end.
  - `.env` / `PLEXUS_SKILLS_DIR` removal doesn't break startup.
  - Fresh DB `initialize()` produces expected schema shape.
- **Manual smoke:**
  - End-to-end chat image-drop → workspace file present, message has base64, frontend renders.
  - Agent calls `edit_file(device_name="server", path=..., ...)` end-to-end.
  - `PATCH /api/devices/.../config` pushes `ConfigUpdate` frame; client re-applies.
  - `fs_policy` flip to `unrestricted` requires typed confirmation.

---

## 13. Net Delta (Estimate)

**Deleted / gone:**
- `/api/files`, `file_store.rs`, `skills_api.rs`
- All 410 handlers (soul×2, memory×2, skills×3)
- Per-user SSRF, soul, memory-as-endpoint
- `update_timezone`, `ERROR:{filename}` sentinel, `RwLock<String>` on prompts
- Duplicate MIME helpers (3 → 1)
- Duplicate edit-match logic (2 → 1)
- Duplicate write/quota logic (3 → 1)
- `PLEXUS_SKILLS_DIR` env var
- Migration soup (~100 lines), `skills` table, `users.soul/memory_text/ssrf_whitelist` columns
- `/api/admin/default-soul`, unused `ErrorCode` variants, unused consts

**Unified:**
- File storage (workspace canonical)
- Tool contract (one schema + `device_name`)
- MCP naming (`mcp_<server>_<tool>`)
- Error types (all in plexus-common)
- Network/SSRF policy (server hardcoded, device whitelist)
- Path validation (absolute for agent, relative for frontend)

**Upgraded:**
- Device config fields first-class (editable + pushed on change)
- System prompt has structured device-status block
- Chat-drop images: workspace file + base64 durability
- `file_transfer` generalizes to device↔device streaming with retry

**Rough code delta:** -1500 to -2000 lines net after accounting for the new consolidation modules. Bigger win is semantic clarity — agent tool surface goes from "8 server + 7 client + N weird-named MCPs" to "6 file + 1 shell + 4 server + N well-named MCPs, all routed by `device_name`."

---

## 14. Explicit Non-Goals (restated from 1.1)

- God-file splits
- `sqlx::migrate!` framework
- Channel adapter trait
- Secret-at-rest encryption for `bot_token` fields
- `TestAppStateBuilder` for test helpers
- `memory.rs` rename

Each is a defensible future pass; keeping them out keeps this cleanup reviewable.
