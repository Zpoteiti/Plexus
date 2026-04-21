# Plexus — Design Decisions

Architectural Decision Records for the Plexus rebuild (M0 → M3).

Each record captures **what** was decided and **why**, not how it was implemented. Implementation lives in per-subsystem specs and the code itself.

These supersede the historical ADR set in the previous Plexus codebase — most decisions are carried forward, but many are simplified, deferred, or reversed based on what we learned.

---

## Conventions

- **ADR-###** numbering is stable. New decisions append to the list; older numbers never repurpose.
- **Status:** `accepted` (locked for implementation) · `deferred` (acknowledged but not scoped into M0–M3) · `rejected` (considered, not taken) · `superseded` (replaced by a later ADR).
- **Decisions are grouped by subsystem**, not strictly chronological, so related choices read together.

---

## 1. Architecture

### ADR-001 · Three-crate workspace

**Status:** accepted
**Context:** The previous Plexus had four crates (`plexus-common`, `plexus-server`, `plexus-client`, `plexus-gateway`). Gateway existed for DMZ + horizontal-scale + edge-cached-frontend scenarios. None of these apply to Plexus's actual deployment profile (solo hosted, up to ~hundreds of users, single server process).
**Decision:** Three crates: `plexus-common`, `plexus-server`, `plexus-client`. No `plexus-gateway`.
**Consequences:** `plexus-server` serves everything: REST API, SSE streams, device WebSocket, frontend static files, JWT issuance. One binary, one port, one deployment artifact. Public deployment puts nginx/Caddy in front for TLS (infrastructure concern, not a Plexus responsibility).

### ADR-002 · Frontend embedded in server binary (prod); Vite + proxy (dev)

**Status:** accepted
**Decision:** In release builds, the React frontend is compiled by `npm run build` and baked into the server binary via `rust-embed`. In dev, `npm run dev` runs Vite on `:5173` with a proxy for `/api/*` and `/ws/device` pointing to the running server on `:8080`.
**Consequences:** Single artifact in prod (one `cargo build --release` produces a deployable binary). Fast dev loop (frontend HMR via Vite, server compiled separately).

### ADR-003 · Browser uses REST + SSE; devices use WebSocket

**Status:** accepted
**Context:** Prior Plexus used WebSocket for browser chat. This required a bespoke frame protocol, reconnect bookkeeping, and ws-fan-out in the gateway crate.
**Decision:**
- **Browser ↔ server:** two endpoints, one per direction.
  - **Inbound** (user → server): `POST /api/sessions/{id}/messages` — fire-and-forget per ADR-013, returns 202 immediately.
  - **Outbound** (server → user): `GET /api/sessions/{id}/stream` — Server-Sent Events. Pushes the agent's replies, tool hints (ADR-017), session_update notifications, and kick events as they happen. `EventSource` in the browser auto-reconnects on drop.
- **Device ↔ server:** WebSocket (unchanged) — devices need bidirectional real-time for tool dispatch; live behind NAT; HTTP is wrong primitive.
- **Discord/Telegram:** via their SDKs (serenity/teloxide).

**Why two endpoints for the browser?** Because POST can't carry the agent's reply — ADR-013 says the POST response returns 202 immediately, long before the LLM finishes. The browser needs a persistent channel to receive whatever the agent eventually produces. Polling wastes requests; WebSocket adds protocol baggage (the whole thing we're dropping for browser). SSE is the cleanest push primitive: one-way, cookie-compatible, native reconnect.

**Consequences:** Drops the browser WS protocol entirely. `EventSource` handles reconnect automatically. Gateway crate becomes unnecessary (ADR-001). Multi-tab-same-session is a broadcast to all SSE subscribers for the session; tab-close is automatic subscriber cleanup.

### ADR-004 · Auth: cookie for browser, bearer for programmatic

**Status:** accepted
**Decision:** Same JWT, two delivery mechanisms. Login returns the JWT + sets an `HttpOnly; Secure; SameSite=Strict` cookie. Browser uses cookie automatically (including for SSE, since EventSource sends cookies). Programmatic consumers (scripts, CLI) use `Authorization: Bearer <jwt>`.
**Consequences:** No client-side token storage bugs in the frontend (the past `localStorage.getItem('token')` vs. Zustand-envelope mismatch cannot recur). Same-origin enables zero CORS friction.

---

## 2. Message Bus & Entrance

### ADR-005 · Single `InboundMessage` shape; no `EventKind`

**Status:** accepted
**Context:** Prior Plexus had `InboundEvent { kind: EventKind::{UserTurn, Cron, Dream, Heartbeat} }`. This `kind` leaked into rate limiting, publish_final branching, and (via a separate `PromptMode` enum) the system prompt builder. One concept, three enums.
**Decision:**
```rust
pub struct InboundMessage {
    channel: String,                        // "discord" | "telegram" | "browser"
    chat_id: String,                        // channel-scoped identifier
    user_id: String,                        // Plexus account this message belongs to (stamped at ingress)
    content: String,                        // already wrapped for non-partner senders
    timestamp: DateTime<Utc>,
    media: Vec<String>,                     // workspace paths
    metadata: serde_json::Value,            // channel-specific escape hatch
    session_key_override: Option<String>,   // "cron:{job_id}", "heartbeat:{user_id}", etc.
}
```
**Consequences:** No `kind`. No `EventKind`. No `PromptMode` branches downstream. Autonomous events are represented as injected user messages into dedicated sessions (ADR-010, ADR-011). One type, one path.

### ADR-006 · `session_key` = override ∨ `{channel}:{chat_id}`

**Status:** accepted
**Decision:** Session identity is computed from the InboundMessage. If `session_key_override` is set (cron/heartbeat/API), use it verbatim. Otherwise compose `format!("{channel}:{chat_id}")`.
**Consequences:** External channel messages get natural per-conversation sessions. Internal synthesizers can route history to isolated sessions while still targeting the original channel for delivery.

### ADR-007 · No `is_partner` field; wrap baked into content at adapter

**Status:** accepted
**Decision:** When a Discord/Telegram adapter receives a message from a non-partner, it wraps content with `[untrusted message from <sender_name>]:` prefix before building InboundMessage. The wrap is the authoritative trust signal; no downstream consumer re-evaluates.
**Consequences:** Agent sees wrap-or-no-wrap in content directly; system prompt teaches the convention once. DB stores the wrapped form — history replay is faithful. No `is_partner` field propagates.

### ADR-008 · No `sender_id` on InboundMessage

**Status:** accepted
**Decision:** `sender_id` is adapter-internal only — the adapter uses it to compare against `partner_id` for the wrap decision, then discards. Not carried on the message. No downstream consumer uses it (no subagent dispatch, no per-sender moderation in v1).
**Consequences:** Smaller struct. If a future feature (moderation, cross-channel identity, subagent dispatch) needs persisted sender identity, it can be added to the DB message row or to `metadata` at that time. "No caller = delete it."

### ADR-009 · `user_id` stamped at ingress (not lazily derived)

**Status:** accepted
**Context:** Earlier draft considered omitting `user_id` and deriving from `{channel}:{chat_id}` at session-creation time. But every ingress point already has user_id in scope (bot identity for Discord/Telegram, JWT claims for REST, job row for cron/heartbeat). Derivation is strictly more code for zero benefit.
**Decision:** InboundMessage carries `user_id`, stamped by whichever adapter/synthesizer built the message.
**Consequences:** No per-message lookup. No failure mode ("what if the config row was just deleted?"). Clear self-documentation.

### ADR-010 · Autonomous flows = user-message injection into dedicated sessions

**Status:** accepted
**Context:** Nanobot pattern. Cron fires → synthesize InboundMessage with `session_key_override="cron:{job_id}"`. Heartbeat Phase 2 → synthesize InboundMessage with `session_key_override="heartbeat:{user_id}"`. Both flow through the normal agent loop as if a user had typed the content.
**Decision:** There is no "autonomous path" in the agent loop. There are only user messages, some of which happen to have been synthesized by an internal service.
**Consequences:** One code path. No `EventKind` branches. No `PromptMode` branches. The agent cannot distinguish "user said X" from "cron synthesized X" — by design.

### ADR-011 · Per-session async lock + pending queue for mid-turn follow-ups

**Status:** accepted
**Decision:** `publish_inbound` maintains `DashMap<session_key, Arc<Mutex<()>>>` and `DashMap<session_key, mpsc::Sender<InboundMessage>>`. When a new InboundMessage arrives:
- If a pending queue exists for this session, enqueue it and return (a prior message is still processing).
- Otherwise, spawn a task that: acquires the session lock, creates the pending queue, ensures DB session row exists, runs the agent turn, drains the pending queue at the end by re-feeding into publish_inbound.
**Consequences:** Per-session serial, cross-session concurrent. Mid-turn follow-ups from the same user naturally queue (no parallel agent tasks on the same session). No long-lived actor tasks — sessions are DB rows + transient lock entries. Race-free via `DashMap::entry().or_insert_with()`.

### ADR-012 · Three external ingress sources + two internal synthesizers

**Status:** accepted
**Decision:**
- **External:** REST (`POST /api/sessions/{id}/messages`), Discord adapter, Telegram adapter. No `session_key_override`.
- **Internal:** cron fire, heartbeat fire. `session_key_override` always set.
**Consequences:** No distinction between "browser" and "direct API" — they're both REST consumers with JWT auth. Internal synthesizers are the only callers that use `session_key_override`.

### ADR-013 · Fire-and-forget ingress; HTTP caller does not wait on agent

**Status:** accepted
**Decision:** `POST /api/sessions/{id}/messages` returns 202 Accepted (or similar) immediately after persisting the user message and spawning the agent processor task. The browser listens on SSE for progress + final.
**Consequences:** HTTP connections are short. Browser disconnect does not cancel agent work. Agent processing runs to completion regardless of caller connection state.

### ADR-014 · Crash recovery is passive — JIT repair at iteration start

**Status:** accepted
**Context:** If the server crashes mid-turn, DB may have an assistant message with unpaired `tool_use` blocks. Most LLM APIs reject history with unpaired tool_use, so the next call would fail.
**Decision:** On every agent-loop iteration, before building context, scan the tail of history for unpaired `tool_use` blocks. For each, insert a synthetic `tool_result` with `{is_error: true, content: "[server restart: tool was not completed]"}`. Then proceed.
**Consequences:** No startup scan, no background worker. Dormant sessions stay dormant. When a session's next inbound message arrives, the repair runs as a no-op-unless-needed pre-pass. Covers crashes AND user-initiated cancellation (ADR-039). Partial completions (1 of 3 tool_uses completed) preserve the successful ones.

---

## 3. Outbound & Channel Delivery

### ADR-015 · Two outbound variants: Hint + Final

**Status:** accepted
**Decision:**
```rust
enum Outbound {
    Hint  { channel, chat_id, kind: HintKind, text: String },
    Final { channel, chat_id, content, media, reply_to, metadata },
}
```
**Consequences:** Hint is ephemeral and channel-discretion; Final is persistent and universal. Channel adapter trait has `deliver_hint` (default: drop) and `deliver_final` (required). New channels implement `deliver_final` only.

### ADR-016 · No token-level streaming

**Status:** accepted
**Context:** Many channels (Discord, Telegram, SMS, email) don't support token streaming natively. Doing it anyway requires bespoke per-channel batch-and-edit logic with rate limits.
**Decision:** LLM calls are non-streaming. Outbound events are (a) mechanical tool-dispatch hints and (b) final completed messages.
**Consequences:** No delta-accumulation buffers. No partial-message rendering in the frontend. No cancel-mid-stream edge cases. Provider layer is simpler (single request, single response).

### ADR-017 · Hints are mechanical, not LLM-narrated

**Status:** accepted
**Decision:** Hints are generated by the agent loop at specific lifecycle points (tool dispatch start), not by the LLM. Example: `"Executing {tool_name} on {device_name}"`.
**Consequences:** Predictable format across channels. Channel adapters format hints identically (or drop them).

### ADR-018 · Interim LLM narration (alongside tool_use) — persisted but not surfaced

**Status:** accepted
**Context:** LLMs sometimes emit text alongside tool_use blocks: *"I'll check the weather. Let me run this command."* followed by the tool_use block.
**Decision:** This interim text is **persisted in DB** as part of the assistant message's content blocks (per ADR-032), but is **NOT emitted as an Outbound::Final** — the user doesn't see it in the chat surface. Only the terminal assistant message (the one with no remaining tool_use blocks) becomes the Final.
**Consequences:**
- **Continuity for the LLM:** on subsequent iterations within the same turn, the history reconstruction (ADR-022) includes the interim text, so the LLM sees its own prior reasoning and stays coherent across multi-step tool chains.
- **Clean user-facing chat:** the UI/channel shows mechanical tool hints (ADR-017) and the final answer. No "thinking aloud" spam between tool calls.
- **Audit trail preserved:** if debugging a bad agent turn later, the full reasoning chain is in DB.

### ADR-019 · Per-channel hint rendering contract

**Status:** accepted
**Decision:**
- **Browser (SSE):** emit `event: hint { kind, text }` on the session's SSE stream
- **Discord:** `sendChatAction("typing")` or ignore (NOT a visible message — avoids spam)
- **Telegram:** `sendChatAction("typing")` (same reasoning)
- **Future channels (SMS, email, etc.):** drop entirely
**Consequences:** Hints add no clutter to persistent channel histories. Only SSE surfaces them as events because the browser chat UI can benefit.

### ADR-020 · Direct replies route by session's channel+chat_id; `message` tool is for cross-channel

**Status:** accepted
**Decision:** When the agent produces a text-only final response, `publish_final` uses the session's own `channel` and `chat_id` (carried from the InboundMessage that created/continued the session). When the agent wants to reach a different channel, it explicitly invokes the `message` tool with target `channel` + `chat_id` read from the Channels section of the system prompt.
**Consequences:** Agent doesn't need to specify routing for the common case. Cross-channel is explicit, not implicit.

---

## 4. Agent Loop

### ADR-021 · Single while-loop, terminate when LLM returns no tool_use blocks

**Status:** accepted
**Decision:** Classical ReAct shape. Each iteration:
1. Check shutdown cancellation
2. Load history from DB, JIT-repair unpaired tool_use (ADR-014)
3. Build context (pure function, ADR-022)
4. Check compaction threshold; compact if needed; continue
4a. Fetch tool schemas from `tools_registry::get_tool_schemas(user_id)` — usually a cache hit; rebuilt lazily on device/MCP state changes (ADR-071)
5. Call LLM (provider handles vision retry internally, ADR-026)
6. Persist assistant response
7. If no tool_use blocks → publish Final, exit
8. Otherwise: for each tool_use, dispatch serially, persist each tool_result
9. Drain pending queue (mid-turn user messages) if any
10. Continue

### ADR-022 · `context::build_context` is a pure function

**Status:** accepted
**Decision:** No DB access, no state-global access. Takes `ContextInputs` as args, returns `Vec<ChatMessage>`. File I/O for `SOUL.md` and `MEMORY.md` is acceptable inside (bounded, pure-ish), but history + skills + channels + devices are loaded by the agent loop and passed in.
```rust
pub struct ContextInputs<'a> {
    soul: Option<&'a str>,
    user: &'a UserIdentity,
    channels: &'a ChannelSummary,
    memory: &'a str,
    devices: &'a [DeviceStatus],
    skills: &'a [SkillInfo],
    history: &'a [Message],
    now: DateTime<Utc>,
}
```
**Consequences:** Testable with synthetic inputs. No mocking of DB or AppState in context tests.

### ADR-023 · Single system prompt shape (no `PromptMode`)

**Status:** accepted
**Decision:** Every turn builds the same system prompt shape: `soul + identity + channels + memory + skills + devices + runtime`. No mode branching for cron/heartbeat/dream — those arrive as normal user messages in dedicated sessions (ADR-010).
**Consequences:** `context.rs` ~half the size of its prior Plexus equivalent. One test surface. The system prompt describes static facts about the user's configuration; dynamic context lives in message history.

### ADR-024 · Skills: always-on full body; conditional name + description

**Status:** accepted
**Decision:** SkillInfo has `always_on: bool`. Skills marked always-on have their full SKILL.md body inlined in the system prompt. Conditional skills appear as one-line entries (`name: description`) with a pointer to load via `read_file(path="skills/{name}/SKILL.md")`.
**Consequences:** Progressive disclosure. Large skill libraries don't bloat every prompt. Agent knows what exists and can pull on demand.

### ADR-025 · `tiktoken-rs` for accurate token counts

**Status:** accepted
**Decision:** Compaction threshold checks use tiktoken-rs, not byte-count heuristics. Required for correctness across different tokenizers.
**Consequences:** Adds `tiktoken-rs` dependency. One compile-time cost for a correctness win.

### ADR-026 · Vision retry lives in the provider layer

**Status:** accepted
**Context:** Some LLMs don't support images. Prior design had `vision_stripped: bool` on session state, persisted across turns.
**Decision:** No session state. On LLM error, if the request contained `image_url` blocks, retry once with them stripped (keep all text blocks, including path-text markers). Return result. No flag propagates.
**Consequences:** ~100 LoC simpler. DB stores full-fidelity messages always. Switching to a VLM mid-session works immediately — no stale flag. Cost: one 500ms retry per image turn on non-VLM.

### ADR-027 · Path-text markers accompany every image attachment

**Status:** accepted
**Decision:** When the adapter adds an image block to user content, it ALSO adds a text block: `"User has uploaded a file to device='server', path='.attachments/...'"`. After vision-strip retry, this text block remains, giving the LLM enough context to reference the file via `read_file` or other tools.
**Consequences:** Non-VLM agents can still reason about uploaded files structurally. VLM agents have redundancy (image + path), which is fine.

### ADR-028 · Two-stage compaction

**Status:** accepted
**Decision:**
- **Stage 1** (user-turn boundary): compact the range `[after system prompt ... before latest user message]` into a single compressed message. Target: 12k tokens.
- **Stage 2** (mid-turn): if history still exceeds threshold after stage 1, compact `[latest user message + accumulated tool/assistant within current turn]` into another 12k-target summary.

**Units clarification:** the 16k threshold and 12k target are **tokens** (measured via tiktoken-rs per ADR-025). This is separate from tool result caps (ADR-076), which are **characters** — roughly 4× smaller in token terms. A max-size tool output (16k chars ≈ 4k tokens) uses ~¼ of the compaction headroom, so ~4 such outputs fit before stage-1 compact fires. Mid-turn accumulation of many tool results is what stage 2 handles.

**Consequences:** Handles both long histories and long agentic runs. Compressed messages are stored in DB with a flag to prevent re-summarization. Stage 2 is rare in practice (needs 30+ tool calls in one turn) but correct when needed.

### ADR-029 · Serial tool dispatch; DB is mid-turn source of truth

**Status:** accepted
**Decision:** Tool calls within a single LLM response are dispatched one at a time, not in parallel. Each tool's `tool_result` is inserted into DB immediately on completion. When all tools in the response have run, the loop continues; next iteration's `build_context` reloads fresh history from DB.
**Consequences:** Order-dependent tool chains (edit file → run file) are safe. No in-memory "current turn" buffer — makes crash recovery trivially correct (ADR-014). LLM sees consistent history every iteration.

### ADR-030 · One hint per tool_use at dispatch time, no end-hint

**Status:** accepted
**Decision:** Immediately before dispatching a tool call, emit one Outbound::Hint. No hint on completion (the next LLM call will incorporate the result; the final message is the summary).
**Consequences:** UI shows activity in order. No "tool X started / tool X finished" noise.

### ADR-031 · Tool failures propagate as `tool_result` error content

**Status:** accepted
**Decision:** All tool failures (timeout, permission, bad args, panic) return a `tool_result` block with `is_error: true` and explanatory content. The agent observes the error in the next iteration and decides recovery. The loop does not break on tool failure.
**Consequences:** Agent can retry, ask the user, or give up. No centralized error-handling for tools.

### ADR-032 · Persist immediately on every state transition

**Status:** accepted
**Decision:** The following events each trigger an immediate DB insert (no batching):
- LLM returns an assistant message (with or without tool_use): insert as role="assistant"
- A tool dispatch completes: insert tool_result as role="tool"
- A user message arrives: insert as role="user"
- Compaction produces a summary: insert as role="system" (or flagged equivalent)
**Consequences:** DB state is always within one insert of the truth. Crash recovery is clean (ADR-014). DB latency (low milliseconds) << LLM latency (seconds), so no perf impact.

### ADR-033 · `publish_final` when: no more tool calls, hard cap, or fatal error

**Status:** accepted
**Decision:** The agent loop emits Outbound::Final in exactly three cases:
1. LLM returns an assistant response with no tool_use blocks (normal completion)
2. Hard iteration cap hit (200)
3. Unrecoverable error (LLM persistent failure after vision-retry)
Otherwise the loop continues.

### ADR-034 · Mid-turn inbound queues; drains at iteration boundary

**Status:** accepted
**Decision:** When a new InboundMessage arrives for a session that is currently processing, `publish_inbound` enqueues into the session's pending queue (created at agent-loop spawn). At the iteration boundary (after tools are persisted, before next build_context), the agent loop drains the queue and persists each as a role="user" message. The next iteration's LLM call sees the new messages naturally.
**Consequences:** Users can redirect mid-turn ("wait, do Y instead") without waiting for the current turn to finish. No special plumbing — just drain at boundary.

### ADR-035 · User stop button: cancel flag + injected user message

**Status:** accepted
**Decision:** Frontend offers a stop button. `POST /api/sessions/{id}/cancel` sets `session.cancel_requested: AtomicBool`. At the next iteration boundary, the agent loop observes the flag, injects a synthetic user message `"[User pressed stop]"` into the pending queue, and exits the loop. DB may end with unpaired tool_use; ADR-014 repair handles it on resume.
**Consequences:** No separate cancel pipeline. Stop produces a natural user-turn boundary the agent observes. Next inbound resumes cleanly with context of the interruption.

### ADR-036 · Hard cap 200 iterations + trap-in-loop detection

**Status:** accepted
**Decision:**
- **Hard cap:** 200. Safety net for infinite-loop bugs.
- **Trap detection:** if the last three tool calls are identical `(name, args_hash)` and consecutive (A-A-A), inject a user-role message: *"You've called `{tool}` with the same args 3 times. Reconsider or ask the user for clarification."* Reset counter on any different call.
- Patterns like A-B-A-B do NOT trigger.
**Consequences:** Cost of LLM runaway is bounded. Agent has a chance to self-correct before hard cap fires.

### ADR-037 · Graceful shutdown observes cancellation token at iteration boundaries

**Status:** accepted
**Decision:** `state.shutdown` cancellation token is observed:
- At the start of each agent-loop iteration
- During LLM call via `tokio::select!`
- During tool dispatch via `tokio::select!`
Once fired, in-flight tools complete (bounded by their own timeout), then the loop exits. No new iteration starts.
**Consequences:** SIGTERM triggers graceful exit. DB ends consistent-modulo-unpaired-tool_use which ADR-014 handles on next inbound.

---

## 5. Tools

### ADR-038 · Shared tool schemas live in `plexus-common`

**Status:** accepted
**Decision:** File tools used by BOTH server and client executors (`read_file`, `write_file`, `edit_file`, `delete_file`, `list_dir`, `glob`, `grep`) have their canonical JSON schemas in `plexus-common/src/tool_schemas/`. Both server and client crates import these.

### ADR-039 · Client-only tools live in `plexus-client`

**Status:** accepted
**Decision:** `shell` (and any future client-only tools) have their schemas in `plexus-client/src/tool_schemas.rs`. Clients report their tool schemas to the server at handshake time via `ClientToServer::RegisterTools.tool_schemas`.
**Consequences:** Server doesn't statically depend on plexus-client. Tool schemas cross the crate boundary via protocol (runtime), not imports (compile).

### ADR-040 · Server-only tools live in `plexus-server`

**Status:** accepted
**Decision:** `message`, `web_fetch`, `cron`, `file_transfer` are plexus-server-owned and defined there.

### ADR-041 · `device_name` routes file tool calls (injected at merge)

**Status:** accepted
**Decision:** Source tool schemas (in `plexus-common/src/tools/`, `plexus-client/src/tools/`, or MCP wraps) are nanobot-shape and **do not include `device_name`**. At session tool-schema-build time, `tools_registry::build_tool_schemas` injects the `device_name` enum (per ADR-071) into the agent-visible schema. Dispatch:
- `device_name="server"` → `workspace_fs` directly
- otherwise → WebSocket `ToolCall` frame to the named device
**Consequences:** Source schemas stay pristine and testable against nanobot fixtures. `device_name` only appears in the post-merge schema the LLM sees. Agent sees `edit_file` not `edit_file_server` vs `edit_file_laptop`.

### ADR-071 · Tools with the same name + schema are merged; `device_name` enum lists install sites

**Status:** accepted
**Context:** Without this rule, if `read_file` exists on server + three devices, the agent would see four separate tools or four overlapping schemas. That defeats the point of the unified tool surface (ADR-041) and blows up the agent's tool-registry cognitive load.
**Decision:** At tool-schema-build time (per session), `tools_registry::build_tool_schemas` deduplicates:

1. Group incoming tool schemas by `(fully_qualified_name, canonical_schema)`.
2. For each group, emit **one** merged schema whose `device_name` enum lists every install site that reported it.
3. If two install sites report the same name but different canonical schemas, REJECT — ADR-049 for MCP collisions; for non-MCP tools, this is a bug (shared tools should have server-owned canonical schemas per ADR-038).

**Applies to:**
- **Shared file tools** (`read_file`, `write_file`, etc.): server schema is canonical (ADR-038). Every connected device reports the same schema. Enum = `["server", <device_1>, <device_2>, ...]`.
- **Client-only tools** (`shell`): schema owned by client (ADR-039), advertised at handshake. Enum = `[<device_1>, <device_2>, ...]` (no "server").
- **Server-only tools** (`message`, `web_fetch`, `cron`, `file_transfer`): single install site, no merge needed, no `device_name` arg in schema.
- **MCP tools** (`mcp_{server}_{tool}`): collision-checked at install (ADR-049); schemas guaranteed identical across sites when install succeeds. Enum lists all install sites of this MCP server.

**Canonical schema comparison:** compare the schema after normalizing whitespace, property ordering, and OpenAI-compatibility transforms. Use a stable JSON canonicalization (e.g. sorted keys, trimmed descriptions).

**Stale-read tolerance:** the agent loop reads `tools_registry` at the start of each iteration (ADR-021 step 4a). A cache invalidation during iteration N may not be reflected in N's LLM call; iteration N+1 will see fresh schemas. Bad tool calls caused by stale reads produce `tool_result { is_error: true }` per ADR-031, and the agent adapts on the next iteration. Tightening this window (generation counters, mid-iteration re-reads) is not worth the complexity — the tool-error pathway is the authoritative correctness guarantee, since devices can disappear mid-dispatch regardless of cache consistency.

**Consequences:** Agent sees one tool per capability, with a clear enum of where it can run. Tool-registry cache invalidates on any device connect/disconnect or config change that affects schema reporting. Collision detection is load-bearing for both MCP (ADR-049) and shared file tools (catches bugs where server and client drift).

### ADR-042 · `edit_file` uses nanobot-derived 3-level fuzzy match

**Status:** accepted
**Decision:** Matcher levels: (1) exact substring, (2) line-trimmed sliding window (handles indentation drift), (3) smart-quote normalization. Multi-match requires `replace_all=true`. Create-file shortcut: `old_text=""` + file doesn't exist → create with `new_text`.
**Consequences:** Same matcher on server and client (lives in `plexus-common`). Tool args: `path`, `old_text`, `new_text`, `replace_all`.

### ADR-043 · Agent tool calls use absolute paths; frontend REST uses relative

**Status:** accepted
**Decision:** Tool arguments carrying file paths MUST be absolute. The system prompt's "Your targets" section lists each target's `workspace_root`. Frontend REST endpoints (e.g., `GET /api/workspace/files/{path:.*}`) accept relative paths since user_id auth supplies the root scope.
**Consequences:** Agent log traces are unambiguous. No "relative to what?" confusion.

### ADR-044 · Workspace is the canonical file store; no parallel file cache

**Status:** accepted
**Context:** Prior Plexus had `/api/files` (ephemeral upload cache, 24h TTL) running parallel to `/api/workspace/files/` (durable user tree). Two storage systems for files caused drift across message-send, context-load, and channel delivery.
**Decision:** Workspace is canonical. No `/api/files`, no `file_store.rs`. Chat-drop images land at `workspace/.attachments/{msg_id}/{filename}` — a reserved directory with 30-day TTL sweep, counts toward quota.
**Consequences:** One file model. All inbound/outbound media flows through workspace paths. Discord/Telegram adapters read workspace files directly for delivery (no staging cache). Device-origin files: the agent uses `file_transfer` to stage to server first, or the server relays via `GET /api/device-stream/{device_name}/{path}` (SSE-compatible for browser display).

### ADR-045 · `workspace_fs` is the single write path server-side

**Status:** accepted
**Decision:** One service module owns path resolution + quota reserve/rollback + skills-cache invalidation + symlink-escape check. All REST handlers + server tools that write to workspace go through it. No independent `tokio::fs::write` calls for user data.
**Consequences:** One bug-fix location for path safety, one place to add quota enforcement, deterministic skills-cache invalidation on any write under `skills/`.

### ADR-046 · All typed errors live in `plexus-common/src/errors/`

**Status:** accepted
**Decision:** `WorkspaceError`, `ToolError`, `AuthError`, `ProtocolError`, `McpError`, `NetworkError`. Each implements `fn code(&self) -> ErrorCode`. HTTP mapping (`ApiError → StatusCode`) lives in `plexus-server` but wraps these. Server layer does NOT define new error types.
**Consequences:** One source of truth for what can go wrong. Wire-level `ErrorCode` enum remains stable across versions. `QuotaError` is flattened into `WorkspaceError` (`UploadTooLarge`, `SoftLocked`).

### ADR-075 · Tool timeouts are decentralized; agent may override where the schema advertises

**Status:** accepted
**Context:** Nanobot's tool timeout model (confirmed empirically). Tools that have legitimately variable duration (shell commands, some MCPs) expose `timeout` as a schema parameter the agent can set within bounds. Tools with bounded scope (file ops, web_fetch, message, cron) enforce fixed internal timeouts with no agent override.
**Decision:**
- **No central dispatcher-level timeout wrapper.** Each tool owns its timeout enforcement in its own `execute()`.
- **Tools expose `timeout` in their schema only when it makes sense.** The agent sees `timeout` as an integer param with documented min/max where exposed.
- **Per-tool defaults for Plexus:**

| Tool | Agent can override | Default | Max |
|---|---|---|---|
| shell | yes | 60s | `device.shell_timeout_max` |
| read_file | no | 30s internal | — |
| write_file | no | 30s internal | — |
| edit_file | no | 30s internal | — |
| delete_file | no | 10s internal | — |
| list_dir | no | 10s internal | — |
| glob | no | 30s internal | — |
| grep | no | 60s internal | — |
| message | no | 30s internal | — |
| web_fetch | no | 30s total, 10s connect | — |
| cron | no | 10s (DB op) | — |
| file_transfer | no | stall-detect: abort if no bytes in 30s | — |
| MCP tools | depends on MCP's own schema | varies | rmcp session timeout |

- **Runaway guardrail** is the iteration hard cap (ADR-036, 200) + trap detection. Not per-call timeouts.

**Consequences:** Simpler dispatch layer. Each tool's timeout is self-documenting in its own code + schema. shell is the primary agent-tunable case; other file-ops and server-only tools pick sensible internal limits. file_transfer's stall-detection covers the unbounded-legitimate-case (10 GB over slow link).

### ADR-076 · Tool result cap: 16k chars global default + per-tool override; head-only truncation

**Status:** accepted
**Context:** Nanobot's pattern. Prevents a single tool run from flooding agent context while giving tools with legitimate high-output needs (file read) room to breathe.
**Decision:**
- **Global default: 16,000 characters** per tool_result (counted via `chars().count()`, UTF-8-aware).
- **Per-tool override via `Tool::max_output_chars()`** default method. Example: `read_file` overrides to 128,000.
- **Head-only truncation.** If output exceeds cap: emit `output.chars().take(cap).collect::<String>() + "\n... (truncated)"`. No head+tail split — errors and useful signal appear at the start of virtually every tool output shape.
- **Truncation helper lives in `plexus-common`** (single implementation, no duplication).

**Units clarification:** this cap is **characters**, not tokens. Roughly 4× smaller in token terms (16k chars ≈ 4k tokens for English/code). Compaction threshold (ADR-028) is in tokens; these are different budgets.

**Consequences:** One tool call can't blow up context. Truncation is centralized and predictable. Future tools with special needs (large binary dumps, wide tables) can override.

### ADR-077 · `Tool` trait pattern with default methods

**Status:** accepted
**Context:** Nanobot uses an abstract base class (`Tool` ABC) with default methods and per-tool overrides. Rust's trait system gives us the same shape natively.
**Decision:**
```rust
// plexus-common/src/tools/mod.rs
pub const DEFAULT_MAX_TOOL_RESULT_CHARS: usize = 16_000;

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// Tool name as it appears in the schema (e.g., "read_file", "shell").
    fn name(&self) -> &str;

    /// JSON Schema for the tool parameters. Nanobot-shape; device_name
    /// is injected at merge time (ADR-041, ADR-071), not here.
    fn schema(&self) -> serde_json::Value;

    /// Per-tool result cap. Default matches global (ADR-076).
    fn max_output_chars(&self) -> usize {
        DEFAULT_MAX_TOOL_RESULT_CHARS
    }

    /// Execute the tool call with validated args and an execution context
    /// (user_id, session_id, device_name, state refs).
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}
```

**Registry shape:** `HashMap<&'static str, Arc<dyn Tool>>` per crate (server + client each register their own). Schema merging at session tool-schema-build time (ADR-071) pulls from both plus cached device advertisements.

**Consequences:** Each tool is a testable unit. Default-methods pattern means tools only override what's different from defaults (most tools just need name/schema/execute). Cross-cutting concerns (truncation, timeout, permission pre-check) can be added via default methods later without breaking implementers.

---

## 6. MCP

### ADR-047 · Shared MCP client in `plexus-common`

**Status:** accepted
**Context:** Both server (admin-installed MCPs) and client (user-installed per-device MCPs) need an rmcp-based MCP client. Prior Plexus had ~150 LoC of duplicated wrapper in both crates.
**Decision:** `plexus-common/src/mcp/` contains the shared `McpSession` + `McpManager` + transport setup (`TokioChildProcess`). Server and client each import.
**Consequences:** Single implementation. Per-site specific bits (server loads config from `system_config`; client applies from `ConfigUpdate`) stay in the owning crate. `rmcp` is already a workspace dependency.

### ADR-048 · MCP tool naming: `mcp_{server}_{tool}`

**Status:** accepted
**Decision:** The MCP wrap step prefixes each MCP-provided tool name with `mcp_<server_name>_<tool_name>`. Nothing else is injected at wrap time — source schema stays unchanged. The `device_name` enum is added later at merge time per ADR-071, consistent with ADR-041.
**Consequences:** Wrap is pure name-rewriting; merge is where cross-site schema comparison + device_name injection happens. Cleanly separates concerns.

### ADR-049 · MCP schema-collision is rejected at install time

**Status:** accepted
**Decision:** Same `mcp_<server>_<tool>` name MUST have identical tool schemas across ALL install sites. On install (`PUT /api/devices/{name}/mcp` for device-level, `PUT /api/server-mcp` for admin-level), the incoming MCP's tools are introspected (10-second timeout for admin server-side via rmcp; already-cached for device-side via `RegisterTools.mcp_schemas`). If a schema differs from any existing install of the same `<server>` name, return 409 Conflict with a structured diff body.
**Consequences:** Never auto-version / suffix. User renames their local install if they want two versions to coexist. Single canonical schema per name.

---

## 7. Devices

### ADR-050 · Device config is first-class + editable

**Status:** accepted
**Decision:** Each device has `workspace_path`, `shell_timeout_max`, `ssrf_whitelist`, `fs_policy`, `mcp_servers` stored on its row. All editable via `PATCH /api/devices/{name}/config`. Server pushes changes to the live device via `ServerToClient::ConfigUpdate` frame.
**Consequences:** No "stored but unreachable" fields. The system prompt's "Your targets" section renders each device's current config directly.

### ADR-051 · `fs_policy=unrestricted` requires typed-name confirmation

**Status:** accepted
**Decision:** Frontend toggle from `sandbox` to `unrestricted` opens a modal requiring the user to type the device name. Matches the account-deletion confirmation pattern.
**Consequences:** No one-click footgun. Explicit opt-in.

### ADR-052 · Server `web_fetch` has hardcoded RFC-1918 block

**Status:** accepted
**Decision:** `web_fetch` unconditionally blocks RFC-1918, link-local (169.254/16), loopback, carrier-grade NAT. No whitelist exists server-side. Private-network fetches must go through a client device with its own `ssrf_whitelist`.
**Consequences:** Server in prod cannot be tricked into probing its own infrastructure. Per-device whitelist applies only to that device's client-side operations (shell subprocess, client MCP network calls).

---

## 8. Autonomous Flows

### ADR-053 · Cron: per-job dedicated session, inherits channel+chat_id at creation

**Status:** accepted
**Decision:** When the agent creates a cron job, the job row stores the current session's `channel` + `chat_id` so the eventual reply lands where the user set it up. `session_key_override = "cron:{job_id}"` isolates each job's history.
**Consequences:** User on Discord says "remind me every morning" → the reminder fires on Discord. Each cron job has an auditable conversation history independent of others.

### ADR-054 · Heartbeat: 2-phase, only Phase 2 goes through the bus

**Status:** accepted
**Decision:**
- **Phase 1**: a standalone LLM call (not through the bus) with a small decision tool. Inputs: `HEARTBEAT.md` + current time. Output: `action: "skip" | "run"` + `tasks` summary.
- **Phase 2** (only if action=run): synthesize InboundMessage with `session_key_override = "heartbeat:{user_id}"`, inject into bus. Normal agent loop runs in the heartbeat session.
**Consequences:** No `PromptMode::Heartbeat` branch — Phase 2 sees the standard system prompt. Heartbeat has its own session per user, so it doesn't pollute chat history.

### ADR-055 · Dream deferred for v1

**Status:** deferred
**Context:** Prior Plexus had Dream as a two-phase background consolidation of history into `MEMORY.md` + skill discovery.
**Decision:** Not in M0–M3. MEMORY.md is maintained inline by the main agent via `edit_file` during conversations. When Dream eventually lands, it will be a separate sidecar module (not on the bus) with its own restricted tool registry, matching the nanobot pattern. Nothing in the rebuild architecture blocks its future addition.
**Consequences:** No `last_dream_at` column, no `dream_phase1_prompt`/`dream_phase2_prompt` system_config keys, no `ToolAllowlist::Only(...)` enum, no `kind` column on `cron_jobs` (system cron kind was only used for dream + heartbeat; heartbeat is a tick loop, not a cron row).

### ADR-056 · No rate limiting in v1

**Status:** accepted
**Decision:** Self-hosted Plexus targets modest scale (hundreds of users on adequate hardware). The only protective behavior: on LLM provider 429, retry twice with exponential backoff, then surface an error to the user. No rate-limit buckets, no counters, no per-user quotas in the bus.
**Consequences:** Simpler ingress. Admin's responsibility to size their LLM provisioning. Future rate limit can be bolted on at the bus layer when a deployment actually needs it.

---

## 9. Persistence

### ADR-057 · Canonical `schema.sql` loaded via `include_str!`

**Status:** accepted
**Decision:** One SQL file at `plexus-server/src/db/schema.sql` contains every `CREATE TABLE` + index + constraint. Server startup runs the whole thing once (`sqlx::raw_sql(include_str!("schema.sql"))`). `IF NOT EXISTS` makes re-runs idempotent.
**Consequences:** No migration framework until first real user. Schema changes during rebuild require dev DB reset (`scripts/reset-db.sh`). When real users land, add `sqlx::migrate!` + proper versioned migrations.

### ADR-058 · Every user-referencing FK has `ON DELETE CASCADE` inline

**Status:** accepted
**Decision:** Cascades defined at table-create time, not via `ALTER TABLE` migrations. Account deletion is a single `DELETE FROM users WHERE id = $1` that cleans up devices, device_tokens, sessions, messages, cron_jobs, discord_configs, telegram_configs automatically.

### ADR-059 · Messages store Anthropic-style content blocks as JSONB

**Status:** accepted
**Decision:** `messages.content JSONB` holds the array of content blocks (text, image, tool_use, tool_result). Both user and assistant messages can be multimodal. Tool messages too.
**Consequences:** No lossy text-only representation. History replay is full-fidelity for VLMs and for non-VLMs (with path-text fallback, ADR-027).

### ADR-060 · No `users.soul`, `users.memory_text`, `users.ssrf_whitelist`

**Status:** accepted
**Decision:** SOUL.md and MEMORY.md are files in the user's workspace, not DB columns. Per-user SSRF whitelist doesn't exist server-side (ADR-052); only per-device whitelists.
**Consequences:** Editable by the agent via file tools without specialty endpoints. Inspectable on disk. Versioned naturally via git if the user cares.

---

## 10. Safety

### ADR-072 · Server is not a code execution environment for agents

**Status:** accepted
**Context:** The server hosts user workspaces on disk — SOUL.md, MEMORY.md, `skills/`, `.attachments/`, arbitrary user-uploaded files. Any of these could contain executable content (a shell script, a Python file, a binary). The agent itself can write such content via `write_file`. The question: can the agent, or the content, cause the server to execute something?
**Decision:** **No.** The agent's server-side tool surface is deliberately restricted to non-executing operations:

- **File tools** (`read_file`, `write_file`, `edit_file`, `delete_file`, `list_dir`, `glob`, `grep`) — byte-level operations through `workspace_fs`. Read and write content, never interpret it.
- **`message`** — delivers text/media to a channel. No execution.
- **`web_fetch`** — HTTP GET/POST with hardcoded RFC-1918 block (ADR-052). Content is returned as bytes; server does not evaluate.
- **`cron`** — schedules future agent invocations. Does not itself execute anything.
- **`file_transfer`** — moves bytes between server and a device. No execution.

Absent, deliberately: `shell`, `exec`, `python`, `eval`, any code-execution tool.

**Consequence:** An agent that writes `rm -rf /` into `~/workspace/evil.sh` cannot trigger its execution on the server. Same for anything in MEMORY.md, SOUL.md, `skills/*/SKILL.md`, `.attachments/`. The server treats all user/agent-provided files as inert data.

**Corollary — server-side MCP subprocesses are the one admin-gated exception.** Admin-installed MCPs (ADR-047) run as `TokioChildProcess` via rmcp. This is intentional code execution, but access is:
- Admin-configured only (`PUT /api/server-mcp`, admin JWT required).
- Not agent-reachable beyond the MCP's declared tool schemas.
- Schema-collision-checked at install (ADR-049).

Admin is trusted. Agent is not. The shape of "admin explicitly installs; agent calls tools through protocol" keeps the blast radius bounded to what the MCP itself exposes.

### ADR-073 · Client-side code execution is sandboxed by default; user-opted to unrestricted

**Status:** accepted
**Context:** The client's purpose is precisely the opposite of the server's — it exists to give the agent code-execution capability on the user's device (shell commands, MCP subprocesses, file writes that users will then run themselves). That necessarily creates a risk surface.
**Decision:** Defense-in-depth with two explicit tiers, user-selected per device:

1. **`fs_policy = "sandbox"` (default).** On Linux, `shell` and client-side MCP subprocesses run inside a `bwrap` jail rooted at the device's `workspace_path`. The jail read-binds `/usr`, `/bin`, `/lib`, `/etc/ssl/certs`, etc. (minimum to make a subprocess function), binds `workspace_path` read-write, tmpfs-mounts everything else. No access to `$HOME`, no access to files outside the workspace, no access to host env beyond a minimal whitelist (`PATH`, `HOME`, `LANG`, `TERM`).
2. **`fs_policy = "unrestricted"`.** Sandbox disabled. Agent runs shell + subprocesses with the client process's full privileges. **Toggle requires typed-device-name confirmation** (ADR-051).

**Platform coverage in v1:** Linux (bwrap) only. Non-Linux clients effectively run unrestricted even if `fs_policy="sandbox"` is set, because the sandbox primitive isn't available. Future: macOS `sandbox-exec`, Windows Job Objects / AppContainer. Not blockers for v1.

**Consequences:**
- **Trust model is explicit.** Server protects itself (ADR-072); client protects the user *with the user's consent.* A user who flips to unrestricted or runs Plexus on a non-Linux platform without a sandbox primitive accepts that the agent runs with their full user permissions.
- **The sandbox is a defense, not a guarantee.** `bwrap` namespace isolation is strong but not unbreakable — notable escapes exist for adjacent kernel bugs, privileged capabilities, or misconfigurations. Plexus documents the risk in the device setup UI.
- **Environment isolation applies even in unrestricted mode for shell.** We strip host env to a small whitelist before exec, so secrets in `$GITHUB_TOKEN` etc. don't leak to agent-run processes. (Inherited from nanobot's pattern.)
- **Future sandbox primitives slot in via `fs_policy` values.** Adding macOS support means adding a `fs_policy="sandbox-darwin"` variant or extending the existing `sandbox` enum. No protocol change.

### ADR-074 · Trust model summary

**Status:** accepted (documentation ADR)
**Context:** The above ADRs define the "what"; this one is the "who trusts whom."
**Decision:**
| Principal | Trusted by | To do |
|---|---|---|
| **Admin** (platform operator) | Plexus itself, all users on this deployment | Install server-side MCPs, configure LLM provider, set rate policies (ADR-056 — none in v1), delete users |
| **User** (Plexus account partner) | Their own resources (workspace, devices, channels) | Manage their devices, their skills, their memory, their integrations, their conversation history |
| **Agent** | The user for their own conversation | Read + write within the user's workspace; execute on the user's sandboxed devices; message through the user's connected channels |
| **Partner** (the human on the other end of a channel conversation) | The agent, for responsiveness | Treated as the user by default when the channel config matches; otherwise treated as untrusted (ADR-007) |

**Hard boundaries:**
- Agents never cross user boundaries (user A's agent cannot read user B's workspace).
- Agents cannot execute code on the server (ADR-072).
- Server never inspects or executes content users upload (treated as inert data).
- Cross-account impersonation via JWT forgery is the primary risk and handled by JWT signing (ADR-004); compromise of `JWT_SECRET` is a catastrophic admin-level concern, documented in deployment material.

**What this explicitly does NOT try to defend against:**
- **The user's own agent going off the rails.** If a user instructs their agent to `rm -rf ~` on their own unrestricted device, the agent will comply. That's a user-ergonomics + sandbox policy question, not a platform security question.
- **Compromised LLM provider.** If the admin-configured LLM starts returning malicious tool calls, the agent will attempt them. In sandbox mode this is bounded to the workspace; in unrestricted, the user has accepted the risk.
- **Partners on shared channels.** If Alice shares a Discord channel with Bob, Bob's untrusted-wrapped messages reach the agent. Wrap + system prompt teach the agent to reject instructions from non-partners (ADR-007). Not a cryptographic guarantee.

---

## 11. Explicit Non-Goals (v1)

Listed here so scope is clear. Each is defensible future work but out of M0–M3.

### ADR-061 · No horizontal scale / multi-server coordination
Single server process is the unit of deployment. Multi-node would require session-affinity routing, distributed locks, leader-elected autonomous tickers. Not needed at Plexus's scale.

### ADR-062 · No subagents / agent-spawning
One agent per session. Nanobot supports subagent dispatch via sender_id — we deliberately dropped sender_id from InboundMessage (ADR-008). Add back when a real use case appears.

### ADR-063 · No Dream (deferred, ADR-055)
See ADR-055.

### ADR-064 · No server-side Whisper/ASR
Voice notes save to workspace as-is. Users wire their own transcription by running whisper.cpp (or similar) on a client device and invoking via shell tool.

### ADR-065 · No last-admin invariant enforced
Admin can delete their own account with a warn log. If they were the only admin, re-bootstrapping requires direct DB access. Acceptable for self-hosted deployments.

### ADR-066 · No frontend test harness (Vitest/RTL/Playwright)
Manual smoke testing in v1. Wire up later if frontend complexity grows.

### ADR-067 · No bulk file operations / file rename endpoint
Single-file ops only. Delete + re-upload serves the rename case. Revisit on user demand.

### ADR-068 · No server-pushed workspace tree invalidation
When an agent writes a file, the open Workspace tab doesn't auto-refresh. User reload or navigate triggers refetch. WS/SSE push can be added if the UX friction is real.

### ADR-069 · No real migrations framework in v1
`include_str!("schema.sql")` with `IF NOT EXISTS` semantics is all. Add `sqlx::migrate!` when first real user arrives.

### ADR-070 · No multi-instance-coordination for heartbeat
Heartbeat tick runs per-process. If two servers run the same DB, both would fire heartbeats. Single-node deployment avoids this. Coordinating across nodes requires leader election or advisory locks — deferred.

---

## Appendix A · Key Design Principles

Distilled from the ADRs, for fast onboarding of new contributors:

1. **Generic over specialty.** If a generic tool (read_file, edit_file) can do the job, never add a specialty tool (save_memory, update_soul).
2. **Workspace is the single source of truth for user files.** No parallel caches. Everything flows through `workspace_fs`.
3. **DB is the single source of truth for conversation state.** No in-memory session actor, no mid-turn buffers. Every state change persists immediately.
4. **Autonomous flows are user messages.** Cron, heartbeat → inject InboundMessage into bus. No `EventKind` branches in the main agent.
5. **One schema per tool name.** Collisions across install sites are rejected, not auto-versioned.
6. **No speculative scaffolding.** Fields without consumers are rejected. Add them back in five lines when a consumer appears.
7. **No rate limiting in v1. No dream in v1.** Admin provisions their LLM; agent maintains MEMORY.md inline.
8. **Pure functions where possible.** `context::build_context`, the fuzzy matcher, `validate_url` — all pure. Testable with synthetic inputs.
9. **Crash recovery is passive.** JIT repair on next activity. No startup scans, no background workers.
10. **Channel adapters are thin.** Platform event → InboundMessage → bus. Agent doesn't know which channel it's on; adapters translate.

---

## Appendix B · What We Explicitly Reversed From the Prior Plexus

For contributors migrating from the old codebase, here's what changed and why:

| Reversed decision | New decision | ADR |
|---|---|---|
| `EventKind::{UserTurn, Cron, Dream, Heartbeat}` | No kind; autonomous = user-message injection | ADR-005, ADR-010 |
| `PromptMode::{UserTurn, Heartbeat, Dream}` | Single system prompt shape | ADR-023 |
| `ToolAllowlist::Only(...)` for Dream | Dropped with Dream | ADR-055 |
| 4-crate workspace (with plexus-gateway) | 3 crates | ADR-001 |
| WebSocket for browser chat | REST + SSE | ADR-003 |
| `InboundEvent.sender_id`, `.identity.is_partner` | Neither field on InboundMessage | ADR-007, ADR-008 |
| Rate limiting in bus | None in v1 | ADR-056 |
| Per-user SSRF whitelist on `web_fetch` | Hardcoded RFC-1918 block | ADR-052 |
| `/api/files` ephemeral cache | Workspace canonical | ADR-044 |
| `vision_stripped` on session state | Retry at provider layer only | ADR-026 |
| Session = long-lived actor task + mpsc inbox | Session = DB row + transient lock | ADR-011 |
| `cascade_migrations` loop in `db/mod.rs` | Canonical `schema.sql` via `include_str!` | ADR-057 |
| Shell schema in `plexus-server/server_tools/` | Client owns; handshake-advertised | ADR-039 |
| File tool schemas in `plexus-server/server_tools/` | `plexus-common/tool_schemas/` | ADR-038 |
| MCP client code duplicated in server + client | Shared in `plexus-common/mcp/` | ADR-047 |

