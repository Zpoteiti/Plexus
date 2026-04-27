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
  - **Outbound** (server → user): `GET /api/sessions/{id}/stream` — Server-Sent Events. On connect, replays recent persisted messages, then switches to live events — see ADR-093 for the replay-then-live pattern. `EventSource` in the browser auto-reconnects on drop, replaying missed events via `Last-Event-ID`.
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
**Decision:** Hints are generated by the agent loop at specific lifecycle points (tool dispatch start), not by the LLM. Example: `"Executing {tool_name} on {device}"`.
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

### ADR-020 · Direct replies route to current session; `message` tool defaults to current session and allows explicit cross-channel override

**Status:** accepted
**Decision:**
- **Text-only direct reply** (no tool call): `publish_final` uses the session's own `channel` and `chat_id` (carried from the InboundMessage). Most common path.
- **`message` tool** (nanobot-aligned): `channel` and `chat_id` are OPTIONAL. If omitted, the tool delivers to the current session's channel + chat_id — same target as a direct reply, but gives the agent access to `media` (attachments) and `buttons` (inline keyboards). If specified, the tool delivers to the named channel + chat_id — cross-channel reach.

**Guidance surfaced to the agent** (via system prompt Operating Notes, nanobot-style):
- Prefer plain text reply for normal conversation turns.
- Use `message` tool when you need to attach files/media (required — `read_file` doesn't deliver files), send inline buttons, or reach a different channel.

**Consequences:** Agent has one clear "emit text" path (direct reply), one clear "emit rich / cross-channel content" path (`message` tool). Cross-channel stays explicit via params. Attachments always flow through the `message` tool. Aligned with nanobot's message-tool contract.

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

### ADR-027 · Path-text markers accompany every chat attachment

**Status:** accepted
**Decision:** When a channel adapter receives an inbound message with one or more attachments, it adds a text block per attachment: `"User has uploaded a file to device='server', path='.attachments/{msg_id}/{filename}'"`. This fires for **every** attachment regardless of MIME type:
- **Images** — adapter adds the path-text block AND an `image_url` block (base64 inline per ADR-059). After vision-strip retry, the path-text block remains so a non-VLM agent still knows the file exists.
- **Non-image files** (PDFs, CSVs, audio, archives, anything else) — adapter adds the path-text block ONLY. There is no `file_url` content block in OpenAI chat completions (ADR-101), so non-image bytes never live inline in `messages.content`. The agent reaches them via `read_file` against the workspace path.

**Consequences:** Non-VLM agents can still reason about uploaded files structurally. VLM agents have redundancy on images (path + base64), which is fine. Non-image files have a single path of access (workspace `.attachments/`) — uniform model regardless of whether the LLM supports vision.

### ADR-028 · Two-stage compaction

**Status:** accepted
**Decision:** Two admin-set keys in `system_config` (ADR-101) drive the trigger:
- `llm_max_context_tokens` — the LLM's context-window size, counted with tiktoken-rs (ADR-025) against the full chat-completions prompt (system + tools + history + new turn).
- `llm_compaction_threshold_tokens` — the headroom that triggers compaction. Default `16000`.

**Trigger:** when `llm_max_context_tokens − tiktoken_count(prompt) < llm_compaction_threshold_tokens`, fire compaction.

**Stages:**
- **Stage 1** (user-turn boundary): compact the range `[after system prompt ... before latest user message]` into a single compressed message. The compaction LLM call uses `max_output_tokens = llm_compaction_threshold_tokens − 4000` (= `12000` at the default), leaving 4k headroom for the next user turn.
- **Stage 2** (mid-turn): if the prompt still trips the trigger after stage 1, compact `[latest user message + accumulated tool/assistant within current turn]` into another summary with the same `max_output_tokens` formula.

**Units clarification:** all the thresholds are **tokens** (tiktoken-rs). Tool result caps (ADR-076) are **characters** — roughly 4× smaller in token terms. A max-size tool output (16k chars ≈ 4k tokens) uses ~¼ of a 16k-token threshold, so ~4 such outputs fit before stage-1 compaction fires. Mid-turn accumulation of many tool results is what stage 2 handles.

**Consequences:** Handles both long histories and long agentic runs. Admin tunes `llm_compaction_threshold_tokens` against their model's behavior — smaller threshold = more frequent compaction with more useful tail history; larger = fewer compaction calls but less room for the next turn. Compressed messages are stored in DB with `is_compaction_summary=true` (ADR-089) to prevent re-summarization. Stage 2 is rare in practice (needs 30+ tool calls in one turn) but correct when needed.

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
**Decision:** All tool failures (timeout, permission, bad args, panic) return a `tool_result` block with `is_error: true` and explanatory content. The agent observes the error in the next iteration and decides recovery. The loop does not break on tool failure. Device-side failures (target client disconnected mid-call, WS frame send failed, heartbeat timeout) are surfaced the same way with `code: device_unreachable` — no server-side retry, fail fast (ADR-096 details the WS-layer mechanics).
**Consequences:** Agent can retry, ask the user, or give up. No centralized error-handling for tools. Trap-in-loop detection (ADR-036) catches agents that retry the same unreachable device repeatedly.

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

### ADR-035 · User stop button: cancel flag + persisted user message

**Status:** accepted
**Decision:** Frontend offers a stop button. `POST /api/sessions/{id}/cancel` sets `session.cancel_requested: AtomicBool`. At the next iteration boundary, the agent loop observes the flag, INSERTs `"[User pressed stop]"` as `role=user` directly into `messages` (per ADR-032's persist-on-every-state-transition rule), and exits the loop. DB may end with unpaired tool_use from the interrupted turn; ADR-014 repair handles it on resume.
**Consequences:** No separate cancel pipeline. The stop marker is a normal user-turn row. Next inbound for this session loads history from DB, sees the stop marker, and the agent picks up the interruption context cleanly — no in-memory state needed to "remember" that the user stopped.

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
**Decision:** File tools used by BOTH server and client executors (`read_file`, `write_file`, `edit_file`, `delete_file`, `delete_folder`, `list_dir`, `glob`, `grep`, `notebook_edit`) have their canonical JSON schemas in `plexus-common/src/tool_schemas/`. Both server and client crates import these.

### ADR-039 · Client-only tools live in `plexus-client`

**Status:** accepted
**Decision:** `exec` (and any future client-only tools) have their schemas in `plexus-client/src/tool_schemas.rs`. Clients report their tool schemas to the server at handshake time via `ClientToServer::RegisterTools.tool_schemas`.
**Consequences:** Server doesn't statically depend on plexus-client. Tool schemas cross the crate boundary via protocol (runtime), not imports (compile).

### ADR-040 · Server-only tools live in `plexus-server`

**Status:** accepted
**Decision:** `message`, `web_fetch`, `cron`, `file_transfer` are plexus-server-owned and defined there.

### ADR-041 · `plexus_device` routes file tool calls (injected at merge)

**Status:** accepted
**Decision:** Source tool schemas (in `plexus-common/src/tools/`, `plexus-client/src/tools/`, or MCP wraps) are nanobot-shape. Routing-only tools (shared file tools, `exec`, MCP) **do not include a `plexus_device` field** in their source schema. At session tool-schema-build time, `tools_registry::build_tool_schemas` injects `plexus_device` (per ADR-071) into the agent-visible schema. Intrinsic-device tools (`file_transfer`, `message`) keep their device fields (`plexus_device` / `plexus_src_device` / `plexus_dst_device`) in source with `enum: ["server"]`; merge extends the enum.

**Why the `plexus_` prefix?** The routing field name must not collide with any tool author's native arg. An MCP tool might legitimately have a `device` argument (e.g., selecting a GPU, audio device, or display). The reserved `plexus_` prefix guarantees the merger's injected property never clobbers a tool's own args.

Dispatch:
- `plexus_device="server"` → `workspace_fs` or the relevant server-side implementation directly
- otherwise → WebSocket `ToolCall` frame to the named device

**Consequences:** Source schemas stay pristine and testable against nanobot fixtures. For routing-only tools, `plexus_device` only appears in the post-merge schema the LLM sees. Agent sees `edit_file` not `edit_file_server` vs `edit_file_laptop`. Reserved name is collision-proof.

### ADR-071 · Tools with the same name + schema are merged; `plexus_device` enum lists install sites

**Status:** accepted
**Context:** Without this rule, if `read_file` exists on server + three devices, the agent would see four separate tools or four overlapping schemas. That defeats the point of the unified tool surface (ADR-041) and blows up the agent's tool-registry cognitive load.
**Decision:** At tool-schema-build time (per session), `tools_registry::build_tool_schemas` deduplicates:

1. Group incoming tool schemas by `(fully_qualified_name, canonical_schema)`.
2. For each group, emit **one** merged schema whose `plexus_device` enum lists every install site that reported it.
3. If two install sites report the same name but different canonical schemas, REJECT — ADR-049 for MCP collisions; for non-MCP tools, this is a bug (shared tools should have server-owned canonical schemas per ADR-038).

**Applies to:**
- **Shared file tools** (`read_file`, `write_file`, etc.): server schema is canonical (ADR-038). Every connected device reports the same schema. Merge injects `plexus_device` as a new property; enum = `["server", <device_1>, <device_2>, ...]`, appended to `required`.
- **Client-only tools** (`exec`): schema owned by client (ADR-039), advertised at handshake. Merge injects `plexus_device`; enum = `[<device_1>, <device_2>, ...]` (no "server", per ADR-072).
- **Server-only tools** (`cron`, `web_fetch`): single install site, no device-routing field.
- **Intrinsic-device server tools** (`file_transfer`, `message`): source schema already has its device field(s) — `plexus_src_device`/`plexus_dst_device` for `file_transfer`, `plexus_device` for `message` — with `enum: ["server"]` as a stub. Merge **extends** each such enum with connected device names — no new property injected.

**Merger detects intrinsic-device fields via an explicit marker, not by enum-shape heuristic.** Each device-routing field in a source schema carries `"x-plexus-device": true` (a JSON Schema extension). The typed helper `plexus_device_field()` in `plexus-common/src/tools/` produces the canonical fragment. The merger scans for this marker when extending enums — avoids the "guess a field is device-routing because its enum happens to be `['server']`" trap.
- **MCP tools** (`mcp_{server}_{tool}`): collision-checked at install (ADR-049); schemas guaranteed identical across sites when install succeeds. Enum lists all install sites of this MCP server.

**Canonical schema comparison:** compare the schema after normalizing whitespace, property ordering, and OpenAI-compatibility transforms. Use a stable JSON canonicalization (e.g. sorted keys, trimmed descriptions).

**Stale-read tolerance:** the agent loop reads `tools_registry` at the start of each iteration (ADR-021 step 4a). A cache invalidation during iteration N may not be reflected in N's LLM call; iteration N+1 will see fresh schemas. Bad tool calls caused by stale reads produce `tool_result { is_error: true }` per ADR-031, and the agent adapts on the next iteration. Tightening this window (generation counters, mid-iteration re-reads) is not worth the complexity — the tool-error pathway is the authoritative correctness guarantee, since devices can disappear mid-dispatch regardless of cache consistency.

**Consequences:** Agent sees one tool per capability, with a clear enum of where it can run. Tool-registry cache invalidates on any device connect/disconnect or config change that affects schema reporting. Collision detection is load-bearing for both MCP (ADR-049) and shared file tools (catches bugs where server and client drift).

### ADR-042 · `edit_file` uses nanobot-derived 3-level fuzzy match

**Status:** accepted
**Decision:** Matcher levels: (1) exact substring, (2) line-trimmed sliding window (handles indentation drift), (3) smart-quote normalization. Multi-match requires `replace_all=true`. Create-file shortcut: `old_text=""` + file doesn't exist → create with `new_text`.
**Consequences:** Same matcher on server and client (lives in `plexus-common`). Tool args: `path`, `old_text`, `new_text`, `replace_all`.

### ADR-043 · Tool path policy — relative paths resolve to the target's personal workspace; absolute required for shared workspaces

**Status:** accepted (revised — nanobot alignment pass)
**Context:** Original decision required absolute paths in all tool args for unambiguity. Matching nanobot's tool surface (its schemas don't distinguish relative/absolute at the schema level) and removing friction for the common case (reading `MEMORY.md`) motivated relaxing this.
**Decision:**

- **`plexus_device="server"` + relative path** → resolved against the caller's personal workspace root, i.e. `{PLEXUS_WORKSPACE_ROOT}/{user_id}/`. Example: `read_file(plexus_device="server", path="MEMORY.md")` reads `/{user_id}/MEMORY.md`.
- **`plexus_device="server"` + absolute path** (leading `/`) → used as given. Required for accessing shared workspaces, because shared workspaces have no implicit relative base from the user's point of view. Example: `read_file(plexus_device="server", path="/production_department/sprint.md")`.
- **`plexus_device="<client>"` + any path** → resolved against the device's `workspace_path` when relative; absolute paths are accepted and, under `fs_policy=sandbox`, must still resolve inside `workspace_path`. Clients are single-workspace, so the distinction is cosmetic.

**Frontend REST endpoints** continue to accept workspace-rooted paths (first segment names the workspace); JWT supplies the user_id scope. No ambiguity because the leading segment is always explicit at the REST surface.

**Consequences:** Agent can reach for `MEMORY.md`, `SOUL.md`, `skills/...` without knowing its own user_id. Shared-workspace access is a minor ceremony (one leading slash + workspace name) that makes cross-workspace calls visually distinct. No "which workspace did they mean?" ambiguity — relative always means personal.

### ADR-044 · Workspace is the canonical file store; no parallel file cache

**Status:** accepted
**Context:** Prior Plexus had `/api/files` (ephemeral upload cache, 24h TTL) running parallel to `/api/workspace/files/` (durable user tree). Two storage systems for files caused drift across message-send, context-load, and channel delivery.
**Decision:** Workspace is canonical for files the agent operates on. No `/api/files`, no `file_store.rs`. Chat-drop attachments land at `workspace/.attachments/{msg_id}/{filename}` (server-side workspace, `{PLEXUS_WORKSPACE_ROOT}/{user_id}/.attachments/...`) — a reserved directory that counts toward quota like any other workspace content. **Note:** this `.attachments/` concept exists only on the server. Client devices have no equivalent — bytes that flow to a client via `file_transfer` or `write_file` land directly in `device.workspace_path` with no special media subdir.
**Consequences:** One file model for agent-accessible files. All inbound/outbound media the agent reads/writes flows through workspace paths. Discord/Telegram adapters read workspace files directly for delivery (no staging cache). Device-origin files: the agent uses `file_transfer` to stage to server first, or the server relays via `GET /api/device-stream/{name}/{path}` (SSE-compatible for browser display).

**Storage by attachment type:**

| Attachment type | Workspace `.attachments/` | DB `messages.content` |
|---|---|---|
| **Image** (jpg/png/webp/gif/...) | yes — bytes written | yes — `image_url` block, base64 data URL inline (ADR-059) |
| **Non-image file** (pdf/csv/audio/archive/...) | yes — bytes written | no `file_url` block exists in OpenAI chat completions (ADR-101); only the path-text marker (ADR-027) lands in DB |

So:
- **Images live in BOTH places.** Workspace copy is for `read_file` / `file_transfer`; DB base64 is the durable conversation-replay source so the LLM request is a pass-through with no marshaling.
- **Non-image files live ONLY in `.attachments/`.** The DB just carries the path-text marker pointing at them. The agent uses `read_file` to access content; the LLM never sees the bytes inline.

If the user or agent later deletes a workspace attachment to reclaim quota:
- **Image deleted:** conversation history still renders + replays via the DB base64. Only the agent's ability to `read_file` that specific path is lost (path-text marker per ADR-027 lets the agent still reason about provenance).
- **Non-image file deleted:** the agent permanently loses access to the bytes (no DB copy to fall back on). The path-text marker remains in history so the agent knows the file existed.

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
| exec | yes | 60s | min(600s, `device.shell_timeout_max`) |
| read_file | no | 30s internal | — |
| write_file | no | 30s internal | — |
| edit_file | no | 30s internal | — |
| delete_file | no | 10s internal | — |
| delete_folder | no | 60s internal | — |
| list_dir | no | 10s internal | — |
| glob | no | 30s internal | — |
| grep | no | 60s internal | — |
| message | no | 30s internal | — |
| web_fetch | no | 30s total, 10s connect | — |
| cron | no | 10s (DB op) | — |
| file_transfer | no | stall-detect: abort if no bytes in 30s; same-device move is atomic (instant) | — |
| MCP tools | depends on MCP's own schema | varies | rmcp session timeout |

- **Runaway guardrail** is the iteration hard cap (ADR-036, 200) + trap detection. Not per-call timeouts.

**Consequences:** Simpler dispatch layer. Each tool's timeout is self-documenting in its own code + schema. `exec` is the primary agent-tunable case; other file-ops and server-only tools pick sensible internal limits. file_transfer's stall-detection covers the unbounded-legitimate-case (10 GB over slow link).

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
    /// Tool name as it appears in the schema (e.g., "read_file", "exec").
    fn name(&self) -> &str;

    /// JSON Schema for the tool parameters. Nanobot-shape; `plexus_device`
    /// is injected at merge time (ADR-041, ADR-071), not here.
    fn schema(&self) -> serde_json::Value;

    /// Per-tool result cap. Default matches global (ADR-076).
    fn max_output_chars(&self) -> usize {
        DEFAULT_MAX_TOOL_RESULT_CHARS
    }

    /// Execute the tool call with validated args and an execution context
    /// (user_id, session_id, plexus_device, state refs).
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}
```

**Registry shape:** `HashMap<&'static str, Arc<dyn Tool>>` per crate (server + client each register their own). Schema merging at session tool-schema-build time (ADR-071) pulls from both plus cached device advertisements.

**Consequences:** Each tool is a testable unit. Default-methods pattern means tools only override what's different from defaults (most tools just need name/schema/execute). Cross-cutting concerns (truncation, timeout, permission pre-check) can be added via default methods later without breaking implementers.

### ADR-078 · Quota: one global value + per-user usage counter

**Status:** accepted
**Context:** Plexus hosts user workspaces on disk. Without bounds, an agent or user can fill the volume and break the service for everyone. Prior Plexus had no quota at all. Nanobot runs single-user and didn't need one.
**Decision:**
- **One global quota value.** Stored in `system_config` under key `quota_bytes`. Admin-editable via admin UI; takes effect immediately for all users. No per-user override. Default: 5 GB.
- **Per-user tracking.** `users.bytes_used` column maintained by `workspace_fs` on every write/delete.
- **Two-layer check before every write (enforced at the single workspace_fs choke point per ADR-045):**
  1. **Lock rule:** if `bytes_used > quota_bytes`, all writes/edits/adds are rejected with `WorkspaceError::SoftLocked`. Only `delete_file` and `delete_folder` are allowed. Lock auto-lifts as soon as a delete pulls usage back under quota — no explicit unlock step.
  2. **Single-op cap:** any single operation bigger than 80% of `quota_bytes` is rejected with `WorkspaceError::UploadTooLarge`. Applies to `write_file` content size, positive `edit_file` delta, and per-file or total folder bytes in `file_transfer` writes whose destination is the server.
- **What counts.** Every byte inside `{workspace_root}/{user_id}/` — SOUL.md, MEMORY.md, `skills/**`, `.attachments/**`, arbitrary user files. No exemptions.
- **Read API.** `GET /api/workspace/quota` → `{ quota_bytes, bytes_used, locked }`. Admin sets global via `PATCH /api/admin/config`.

**Consequences:** One admin knob for all users; simple mental model. One enforcement choke point. Predictable degradation — "workspace full, delete files to continue" — surfaced uniformly to agent (as a tool error per ADR-031) and UI (as a lock flag + error variant).

### ADR-079 · Nightly `du`-based quota reconciliation

**Status:** accepted
**Context:** `users.bytes_used` is maintained by workspace_fs on every write. Drift can occur if a write succeeds but the DB update fails (crash between the two), or if a bug adds a write path that bypasses workspace_fs.
**Decision:** A tokio background task runs daily at 03:00 server local time. For each user, runs `du -sb {workspace_root}/{user_id}` and overwrites `users.bytes_used` with the result. Drift above 1 MB from the prior value is logged as a `WARN` — signal that a write path missed workspace_fs.
**Consequences:** At 1K users, sequential walk completes in a small fraction of a minute (bounded by disk I/O). Drift warnings surface bypass bugs without blocking the fix. No real-time accuracy cost: the counter is eventually correct.

### ADR-080 · Chat-drop attachments degrade gracefully under quota lock

**Status:** accepted
**Context:** A user can hit their quota mid-conversation, then send a Discord/Telegram message with an image attachment. The attachment write would hit `SoftLocked`. Dropping the entire message would lose the user's text and make the agent miss the turn.
**Decision:** When a channel adapter receives an inbound message with attachments while the user is over quota:
- The text portion of the message is delivered normally to the session.
- Each attachment is dropped entirely — no workspace file is written AND no base64 `image_url` block is inserted into `messages.content` (the DB-side of ADR-059 is also skipped). The agent sees no image at all for that message.
- A system note is appended to the user's text block: `[attachment skipped: workspace over quota]`.

The agent sees the note in context, can reference it in its reply, and the user can delete files and resend.
**Consequences:** Messages are never lost wholesale. The "you are over quota" signal surfaces through the conversation itself, not as an out-of-band error. Identical note format across channels.

### ADR-081 · No server-side `.attachments/` sweeper — users manage their own quota

**Status:** rejected (initially proposed as a 30-day TTL sweeper; withdrawn)
**Context:** Chat-drop images land in `{workspace_root}/{user_id}/.attachments/{msg_id}/{filename}` (ADR-044). Without cleanup, these accumulate monotonically and consume quota. A tokio background sweeper (every 6 hours, 30-day mtime threshold) was proposed.
**Decision:** No server-side sweeper. The user is responsible for managing their own workspace usage. If `.attachments/` fills their quota, the soft-lock behavior from ADR-078 surfaces the problem through the UI (`GET /api/workspace/quota` shows `locked: true`) and through agent tool errors (`WorkspaceError::SoftLocked`). From there the user — or the agent, on the user's behalf — deletes old attachments via the workspace browser or `delete_file` / `delete_folder` tools.
**Consequences:**
- Zero server-side auto-deletion. Every byte on a user's workspace is there because the user or their agent put it there and hasn't removed it.
- Simpler server — no background task, no drift between filesystem mtimes and DB `bytes_used`, no ordering concerns with in-flight conversations.
- Pairs cleanly with base64-in-DB (ADR-059): even if the user aggressively cleans `.attachments/` to reclaim quota, conversation history still renders and replays.
- Users who want automatic retention can build it via the agent + cron (ADR-053) — e.g., "every Sunday, delete attachments older than 30 days." That's a user-level policy, not a platform behavior.

### ADR-082 · SKILL.md format + write-time validation

**Status:** accepted
**Context:** Skills are metadata + markdown instructions; the loader (ADR-024) needs a machine-readable format for each skill's name, description, and always-on status.
**Decision:**
- **Format:** YAML frontmatter at the top of SKILL.md, then markdown body. Mirrors Claude Code / nanobot convention.
  ```markdown
  ---
  name: weekly-digest
  description: Summarize last 7 days of Discord into MEMORY.md
  always_on: false
  ---
  ...markdown body...
  ```
- **Required frontmatter fields:** `name` (string), `description` (string).
- **Optional frontmatter fields:** `always_on` (boolean, defaults to `false`).
- **Folder name must match frontmatter `name`.** A skill at `skills/weekly-digest/SKILL.md` MUST have `name: weekly-digest` in frontmatter. Mismatch is invalid.
- **Write-time validation.** `workspace_fs` runs the SKILL.md validator ONLY when the destination path matches `skills/*/SKILL.md` (exactly one level deep, exact filename). Writes to `skills/{name}/FORMS.md` or any other supporting file pass through untouched.
- **On validation failure:** write is rejected with `WorkspaceError::InvalidSkillFormat`. The agent/user must fix the file before re-saving, or save under a different filename (which won't be scanned).

**Consequences:** Malformed SKILL.md files can never exist in a scanner path; the loader never has to handle invalid input at read time. A skill's identity is its folder — displayed name and storage path can't diverge.

### ADR-083 · Skill discovery scans exactly one level deep

**Status:** accepted
**Decision:** At agent-loop start, the skills loader enumerates `skills/*/SKILL.md` — exactly one level deep. Any SKILL.md at `skills/foo/bar/SKILL.md` or deeper is NOT discovered. Supporting files can live at any depth under `skills/{name}/` (e.g. `skills/pdf-skill/scripts/fill_form.py`); only the top-level SKILL.md drives discovery.
**Consequences:** Flat, predictable skill namespace. No recursion cost at load time. Skill authors organize the internals of their folder however they like — nested scripts, reference docs, assets, all invisible to the scanner.

### ADR-084 · Skill install paths: user browser + agent `file_transfer`

**Status:** accepted
**Context:** Skills need a path from "somewhere external" to `skills/{name}/` on the server workspace. Prior Plexus considered a dedicated `install_skill` server tool that would clone from git URLs or unpack tarballs.
**Decision:** Two paths, both reusing existing infrastructure:
1. **User upload/edit via the browser.** The frontend edits workspace files through the standard `/api/workspace/files/{path}` REST surface. Users can drop in a pre-authored SKILL.md, flip `always_on`, or manage supporting files. All writes go through workspace_fs → quota + SKILL.md validation apply.
2. **Agent `file_transfer` from a connected client.** Typical flow: user installs the skill on a client machine via the skill author's installer (e.g. `npx plexus-skills-install pdf-skill` on their laptop). The user then tells the agent to install it. Agent uses `file_transfer` to copy the files from the client workspace into `{workspace_root}/{user_id}/skills/pdf-skill/` on the server. Same quota + validation rules.

Rejected: a dedicated `install_skill` server tool. Would require URL allowlisting, tarball-security handling, and a private-repo auth story. The `file_transfer` pattern reuses the existing sandbox + credential model on the client side, leaving server surface minimal.
**Consequences:** One fewer server tool. No network-fetching code on the server. Skills can originate from any source (git, npm, custom installers, hand-authored) as long as they end up on a connected device before transfer.

### ADR-085 · Skills cache mirrors `tools_registry`

**Status:** accepted
**Decision:** `workspace_fs` maintains a per-user skills cache: `DashMap<user_id, Vec<SkillInfo>>`. Populated lazily at agent-loop start (when `ContextInputs.skills` is assembled) if the entry is absent. Invalidated by any write/delete under `skills/` via the single-write-path guarantee (ADR-045). Stale-read tolerance matches ADR-071: a single turn may see an outdated skill list, and the agent self-corrects on the next iteration.
**Consequences:** One parse per skill per cache lifecycle. Minimal overhead on the hot path (context build). Cache consistency bounded by one turn — same envelope as the tools cache.

### ADR-086 · `delete_folder` shared tool (recursive, no flag)

**Status:** accepted
**Context:** Server has no shell (ADR-072), so without a dedicated primitive, deleting a folder requires N `delete_file` calls. Painful for skill uninstall (several supporting files) and general workspace cleanup. Folder deletion via the workspace browser has the same problem.
**Decision:** New shared tool `delete_folder(device, path)`. Always recursive — deletes the folder and every file/subfolder inside. No flag; a non-recursive variant (`rmdir` on empty dirs only) is too niche for v1.
- **Schema in `plexus-common/src/tools/`** alongside the other shared tools (ADR-038). `device` enum is injected at merge time (ADR-071).
- **Implementations in both `plexus-server` and `plexus-client`.**
- **Server implementation** routes through workspace_fs: sums bytes to be deleted recursively, calls `tokio::fs::remove_dir_all`, applies one `bytes_used -= total` DB update, invalidates the skills cache if any path was under `skills/`. Lock auto-lifts if this brings usage back under quota.
- **Client implementation** is bounded by the client's `fs_policy`. In `sandbox` mode, removal is restricted to inside `workspace_path`. In `unrestricted` mode, it follows whatever path the agent provides.
- **Rejects** if `path` is a file (error directs to `delete_file`) or does not exist.

**Consequences:** Shared tool count goes from 7 to 8. Clean folder-uninstall story for skills and general cleanup. Blast radius is bounded to the user's own workspace (server side) or the client's sandboxed workspace (client side with `fs_policy=sandbox`).

### ADR-087 · `file_transfer` unified with `mode`; folder semantics are recursive

**Status:** accepted
**Context:** Originally `file_transfer` was a cross-device-only copy primitive. A separate `move_file` was considered for same-device rename. Keeping them separate felt cleaner conceptually, but a unified tool is fewer tool slots for the agent to learn and reuses the cross-device byte-moving machinery for all file relocations.
**Decision:**
- **Schema: five required fields** — `plexus_src_device`, `src_path`, `plexus_dst_device`, `dst_path`, `mode`. `mode` enum: `"copy" | "move"`. The two device fields use the reserved `plexus_` prefix (per ADR-041) with source stub `enum: ["server"]`; merge extends.
- **Behavior matrix:**
  - Same-device `copy`: native filesystem copy on that device.
  - Same-device `move`: atomic rename (`tokio::fs::rename`).
  - Cross-device `copy`: server orchestrates streaming pull-and-push over the device WebSocket; source remains intact.
  - Cross-device `move`: same stream copy, then delete source only on successful write. If delete fails after a successful copy, both copies exist and the tool result flags a warning. The inverse (neither copy exists) cannot happen — we order copy-then-delete.
- **Folder semantics.** If `src_path` points to a folder, the operation is recursive. Same-device folder moves remain atomic (single directory-entry rename). Cross-device folder transfers stream each entry; mid-transfer failure triggers partial-dst cleanup.
- **Rejection cases.** `dst_path` already exists → reject (no implicit overwrite). `src_path` does not exist → reject. Symlink-outside-workspace checks apply per each side's `fs_policy`.
- **Quota.** Applies when `plexus_dst_device="server"`. Single-op cap (ADR-078) uses total bytes being written (folder sum for recursive). Move from server refunds on successful delete.
- **SKILL.md validation (applies to BOTH single-file AND folder transfers).** Before any bytes move, the server enumerates every destination path the transfer would produce. For each path that would match `skills/*/SKILL.md` (exactly one level deep, exact filename — same rule as ADR-082), the validator runs against the source content.
  - **Single-file transfer:** if `dst_path` matches `skills/*/SKILL.md` and content is malformed → reject the transfer; no bytes land.
  - **Folder transfer:** the server pre-scans the source tree and identifies every file whose final dst path would match `skills/*/SKILL.md`. It validates ALL such files up-front. If **any** is malformed, the **entire transfer** is rejected atomically — no partial copy lands. This closes the gap where recursive folder transfer would otherwise admit invalid skills for later load-time discovery.
  - Non-SKILL.md files and any files outside the `skills/` tree are untouched by this validator — they transfer normally.

**Consequences:** One tool covers rename, move, copy, install-from-client, and cross-device staging. Agents learn one schema. No separate `move_file` tool. `file_transfer` remains server-owned (ADR-040) because only the server can orchestrate cross-device byte streaming, but its targets can be any connected device including the server itself.

### ADR-088 · `write_file` implicitly creates parent directories

**Status:** accepted
**Context:** Server has no shell, and the shared tool surface has no explicit mkdir. Without auto-creation, saving `skills/new-skill/SKILL.md` would require a precondition step (create folder) that doesn't exist as a tool call.
**Decision:** `write_file(path, content)` applies `mkdir -p` semantics on the path's parent directory — equivalent to `tokio::fs::create_dir_all(path.parent())` before the write. Behavior identical on server and client. Subject to the normal workspace-bounds checks (`fs_policy`) and quota guardrails.
**Consequences:** Agents and users never have to think about folder creation. Saves `skills/my-new-skill/SKILL.md` in a single call. Empty folders don't exist as first-class entities — they're always a byproduct of some file living there. Deleting the last file leaves the folder behind (harmless, `delete_folder` can clean up later).

---

## 6. MCP

### ADR-047 · Shared MCP client in `plexus-common`

**Status:** accepted
**Context:** Both server (admin-installed MCPs) and client (user-installed per-device MCPs) need an rmcp-based MCP client. Prior Plexus had ~150 LoC of duplicated wrapper in both crates. MCP advertises three capability surfaces — tools, resources, prompts — and Plexus exposes all three uniformly to the agent (matches nanobot's pattern).
**Decision:** `plexus-common/src/mcp/` contains the shared `McpSession` + `McpManager` + transport setup (`TokioChildProcess`). Server and client each import. On connect to any MCP server, the manager calls `list_tools()`, `list_resources()`, and `list_prompts()` and registers wrappers for each into the per-user tool registry (naming convention in ADR-048).
**Consequences:** Single implementation. Per-site specific bits (server loads config from `system_config`; client applies from `ConfigUpdate`) stay in the owning crate. `rmcp` is already a workspace dependency. The agent sees a flat list of callable entries — it never branches on "is this a tool, resource, or prompt", just on the wrapped name.

### ADR-048 · MCP wrapping — tools, resources, prompts as tool-registry entries

**Status:** accepted
**Decision:** The MCP wrap step turns each capability advertised by an MCP server into a tool-registry entry. Three name formats, mirroring nanobot's typed-infix convention exactly:

| Surface | Wrapped name | Action when called |
|---|---|---|
| Tool | `mcp_<server>_<tool_name>` | Forwards to MCP `call_tool(name, args)` |
| Resource | `mcp_<server>_resource_<resource_name>` | Forwards to MCP `read_resource(uri)` |
| Prompt | `mcp_<server>_prompt_<prompt_name>` | Forwards to MCP `get_prompt(name, args)` |

The typed infixes (`_resource_` / `_prompt_`) make cross-surface name collisions impossible by construction (a tool named "search" and a resource named "search" wrap to different names). Tools stay unprefixed for back-compat with the original ADR-048 convention.

Source-schema handling per surface:
- **Tool:** the MCP-provided `input_schema` is taken as-is. No injection at wrap time.
- **Resource:** the wrapper's `input_schema` is auto-generated from the resource's URI. Static URIs produce `{type: object, properties: {}, required: []}` — zero-arg call. URI templates (`notion://page/{page_id}`) are parsed and each `{var}` becomes a required string property; the wrapper substitutes at call time before invoking `read_resource` (Plexus divergence from nanobot — see ADR-099).
- **Prompt:** the wrapper's `input_schema` is auto-generated from the prompt's `arguments` array (each argument → property; required-flag honored).

Merge-time injection (ADR-071) is uniform across all three: `plexus_device` is added with the install-site enum, regardless of surface.

**Prompt output convention:** `get_prompt` returns a list of `PromptMessage` objects. The wrapper concatenates the text content of every message with `"\n"` and returns the resulting string as the `tool_result.content` (matches nanobot `mcp.py:408–421`). Non-text content blocks are stringified via Rust `Display`. Empty result → `"(no output)"`. The wrapped result is then prefixed with `[untrusted tool result]: ` per ADR-095.

**Consequences:** Wrap is pure name-rewriting + schema-shape generation; merge is where cross-site schema comparison + `plexus_device` injection happens. Cleanly separates concerns. The reserved `plexus_` prefix on the routing field ensures we never clobber an MCP capability's own args, even if the MCP author used a field named `device`. The agent learns three name patterns and treats them uniformly thereafter.

### ADR-049 · MCP collision rejection — server orchestrates DB cleanup + corrective config_update

**Status:** accepted
**Decision:** Three distinct rejection cases, all handled by the same server-orchestrated cleanup flow:

1. **Within-server cross-surface or intra-surface dup.** If the same MCP server advertises two capabilities that wrap to the same name — two tools named `search`, or any internal duplicate — the install is rejected. (Cross-surface collisions like tool `search` vs resource `search` are impossible by ADR-048's typed infix, so this rule fires only on within-surface dups, which indicate a malformed MCP server.) Plexus diverges from nanobot here: nanobot silently overwrites (`registry.py:19–22`); Plexus rejects so the agent never sees a half-registered MCP.
2. **Cross-install-site schema drift.** Same wrapped name (e.g. `mcp_minimax_web_search`) MUST have an identical source schema across every install site. If any schema differs from an existing install of the same `<server>` name, the new registration is rejected.
3. **Spawn failure on the client side** (ADR-105). The MCP subprocess failed to start, exited during `list_tools/resources/prompts`, or hit the 30-second startup timeout. Same rejection treatment as collisions.

**The server is the orchestrator** — when any of the three fires:

a. Server detects the rejection condition during `register_mcp` processing (cases 1, 2) or via the `spawn_failures` field on `register_mcp` (case 3, see PROTOCOL.md §3.5).
b. Server **removes** the offending entry from the device's `mcp_servers` JSONB on `devices` (case 3) or from `system_config.server_mcp` (cases 1, 2 at admin scope).
c. Server pushes a corrective `config_update` over WS to the client, which then tears down the rejected MCP's subprocess locally per the worker queue in ADR-105.
d. Server emits a `mcp_rejected` event on the per-user SSE channel (ADR-106) so the frontend shows the user a clean error: *"GOOGLE was removed from mac-mini. Reason: schema_collision (Google Search input_schema differs from admin-installed GOOGLE)."*

For the admin-side path (`PUT /api/admin/server-mcp`), case 1/2 still fires as `409 Conflict` synchronously on the HTTP request — admin sees the diff in the response body and chooses how to resolve. Device-side rejection is asynchronous (because the spawn happens after `PATCH /api/devices/{name}/config` returns), which is why we need the SSE channel.

**Coarse-grained removal:** if any tool/resource/prompt within an MCP server triggers rejection, the **whole MCP server** is removed from config — not just the offending capability. Simpler implementation, simpler mental model. User re-adds with a tighter `enabled` filter (ADR-100) or a renamed server if they want partial coexistence.

**Consequences:** Never auto-version / suffix. User renames their local install if they want two versions to coexist. Single canonical schema per wrapped name. Within-server dups, schema drift, and spawn failures all surface to the user via the same channel (per-user SSE) with a `reason` discriminator.

### ADR-099 · MCP resource templates — URI placeholders are surfaced as schema properties

**Status:** accepted
**Context:** MCP resources can be either static URIs (`notion://workspace/index`) or URI templates with placeholders (`notion://page/{page_id}`). Nanobot wraps both shapes identically: the URI is stored verbatim with empty `properties` and the wrapper takes no args (`mcp.py:223, 227–231, 256`). For static URIs this works; for templates, the agent has no way to pass `{page_id}` and the resource is effectively dead weight.
**Decision:** At wrap time, parse `{var}` placeholders out of the resource's URI template using a simple `\{(\w+)\}` regex. For each placeholder, inject one required string property into the wrapper's `input_schema`. At call time, substitute the agent-supplied values back into the URI before invoking `read_resource`. Static URIs (no placeholders) keep the zero-arg wrapper shape.

Worked example. MCP resource with URI template `notion://page/{page_id}` → wrapper schema:
```json
{
  "name": "mcp_notion_resource_page",
  "input_schema": {
    "type": "object",
    "properties": { "page_id": { "type": "string", "description": "URI template variable: page_id" } },
    "required": ["page_id"]
  }
}
```
Agent calls `mcp_notion_resource_page(page_id="abc")` → wrapper computes `notion://page/abc` → `read_resource("notion://page/abc")` → returns the resource content as `tool_result`.

**Consequences:** Templated resources become first-class agent capabilities (Plexus divergence from nanobot, justified by the meaningful UX win). Implementation is small (~30 lines in the wrap step). If a template variable name collides with `plexus_device` (the reserved merge-time field), wrapping fails at install time with a clear error — MCP author renames the placeholder. No support for advanced URI Template syntax (RFC 6570 — query strings, fragments, etc.); only simple `{var}` substitution. If a real MCP needs more, we revisit.

### ADR-100 · MCP `enabled` filter applies uniformly across tools, resources, prompts

**Status:** accepted
**Context:** Nanobot's `enabledTools` config filters `list_tools()` output but does not filter resources or prompts (`mcp.py:511–540` vs `553–577`). Asymmetric — the user can suppress noisy tools but not noisy resources from the same MCP.
**Decision:** Each MCP server config carries an optional `enabled: [<wrapped_name_pattern>...]` field (single allow-list, glob-style patterns matched against the post-wrap name). When present, only matching wrapped entries are registered, regardless of surface. When absent, every advertised capability registers (default-allow). Nanobot's `enabledTools` is renamed to `enabled` in Plexus configs; conversion is mechanical for users importing nanobot configs.

Examples:
- `enabled: ["mcp_notion_*"]` → all notion entries (tools, resources, prompts).
- `enabled: ["mcp_notion_search", "mcp_notion_resource_*"]` → the `search` tool plus every resource.
- `enabled: ["mcp_*_resource_*"]` → every resource from every MCP, no tools or prompts.

**Consequences:** Single mental model — one config field, one filter, three surfaces. Plexus divergence from nanobot, justified by symmetry. Users who want nanobot's tools-only behavior write `enabled: ["mcp_<server>_*"]` excluding the resource/prompt infixes — slightly more verbose but explicit.

### ADR-105 · MCP subprocess lifecycle on plexus-client

**Status:** accepted
**Context:** plexus-client manages user-installed MCP subprocesses on each device. The lifecycle has to handle: initial spawn at handshake, additions and removals via `config_update`, subprocess crashes, schema drift after recovery, `enabled` filter changes, WS reconnects, and concurrent activity from parallel tool dispatch + config edits — all while remaining diagnostically useful when something breaks. This ADR locks the design after a Codex-driven review found 8 issues in an earlier draft.
**Decision:**

#### Per-MCP state model

```
process_state:  Spawning  →  Alive(session, schemas)  ←→  Dead(last_error, schemas)
                              │                                │
                              └── (process exit only) ─────────┘

  Spawning  → on `list_tools/resources/prompts` success → Alive
  Spawning  → on startup timeout (30s) / spawn failure  → not in map (cleaned up via ADR-049 path)
  Alive     → on subprocess unexpected exit              → Dead
  Dead      → on next config_update spawn attempt        → Spawning → Alive
  Alive     → on config_update remove                    → teardown → not in map
```

`Dead` retains the last successful `schemas` so the agent's tool list (server-side `register_mcp` snapshot) stays stable across crashes — the only no-op transition that does NOT trigger `register_mcp` (B2 design: keep registered, error on call with diagnostic content). Calls to a Dead MCP return `tool_result(is_error=true, code='mcp_unavailable', content="MCP <name> is not running. Last error: <subprocess exit + stderr tail>. Reconfigure via Settings → Devices on the web UI.")`.

#### Worker queue — full client-side serialization

All state-mutating work runs on a **single tokio worker task** that pulls from one queue:

```
WS reader (cheap, never blocks):
   ├─ ping → respond pong immediately
   ├─ pong → mark heartbeat OK
   ├─ binary frames → route to active transfer slot (in-flight tool call's IO)
   └─ tool_call / config_update → push to worker queue

Worker (single tokio task, processes one item at a time):
   ├─ tool_call → dispatch → await → send tool_result
   └─ config_update → reconcile MCP set → maybe send register_mcp
```

This eliminates transition races (Alive↔Dead during dispatch, spawn-vs-remove, rapid config edits) without generation counters or per-MCP locks. Trade-off: one device's tool calls don't run concurrently across sessions — chat's 30-second `exec` blocks heartbeat's `read_file` for 30s. Acceptable at Plexus scale (ADR-061 — hundreds of users, 1–2 active sessions per user typical). Heartbeat (`ping`/`pong`) and binary frames bypass the queue so `exec` doesn't trip the 70s heartbeat timeout (PROTOCOL.md §1.4).

#### Initial spawn (A1, eager at handshake)

On `hello_ack`, the worker spawns every configured MCP **in parallel** (each one independent — no cross-cancellation):

```rust
let mut spawns: FuturesUnordered<_> = configs.iter()
    .map(|cfg| async move { (cfg.server_name.clone(), spawn_mcp(cfg).await) })
    .collect();

let mut alive = Vec::new();
let mut failures = Vec::new();
while let Some((name, result)) = spawns.next().await {
    match result {
        Ok((session, schemas)) => alive.push((name, session, schemas)),
        Err(e) => failures.push((name, e)),
    }
}
```

`spawn_mcp` has a **30-second startup timeout** covering subprocess fork + initial rmcp handshake + `list_tools/resources/prompts`. Past 30s → SpawnError, MCP doesn't enter the map. `FuturesUnordered` keeps healthy MCPs from being cancelled when one fails — `try_join_all`'s wrong-failure-model semantics that the prior draft used.

After all results collect, the worker sends one `register_mcp` frame containing both `mcp_servers` (successful spawns) and `spawn_failures` (failed ones). Server processes both fields:
- `mcp_servers` → register tools (collision check applies per ADR-049).
- `spawn_failures` → same treatment as collision rejection per ADR-049: remove from `devices.mcp_servers`, push corrective `config_update`, emit `mcp_rejected` SSE event.

#### Config_update — diff and reconcile (D = match A1)

When a `config_update` arrives, the worker:

1. **Diff** `new_config.mcp_servers` against the current local map.
2. **Spawn** any newly-listed servers via the same `spawn_mcp` flow as initial handshake (`FuturesUnordered`, 30s timeout, capture failures).
3. **Teardown** any locally-running servers no longer in config — forceful kill (ADR-105 teardown details below).
4. **Re-introspect** if the schemas of any unchanged server might have drifted (Dead MCP getting respawned: fresh `list_tools/resources/prompts` runs naturally as part of `spawn_mcp` — we always have fresh schemas after a successful spawn).
5. **Rebuild the registration snapshot** from current state:
   ```
   snapshot = ⋃ across all (Alive ∪ Dead) MCPs:
                 { schemas filtered by that MCP's `enabled` list }
   ```
6. **Compare** new snapshot to last-sent. **Send `register_mcp`** if and only if the snapshot changed.

Single algorithm covers every reason the snapshot might shift: subprocess added, removed, schema drifted on recovery, **`enabled` filter edited** (ADR-100 — filter changes ARE schema changes from the server's POV). Worker doesn't branch on which case fired.

#### Crash recovery (B2 — keep registered)

When an Alive subprocess exits unexpectedly:
- Worker observes the `Child::wait()` future resolving with non-zero exit + stderr tail.
- Transition Alive → Dead, retaining the cached schemas.
- **No `register_mcp` change** (snapshot didn't shift; schemas stayed). Server cache stays warm.
- Tool calls to this MCP return `mcp_unavailable` with the diagnostic content above.

Recovery requires a fresh `config_update` from the user (e.g. they re-save device config in the frontend after fixing the underlying issue). On config_update, the worker re-runs `spawn_mcp` for any Dead entry whose config is still present; if successful, Dead → Alive, snapshot rebuilds, possibly sends `register_mcp` (only if the fresh schemas differ from cached, e.g. the user updated the underlying MCP package version).

#### Teardown — forceful kill, cross-platform

```rust
async fn teardown_mcp(child: Child, io_pumps: Vec<JoinHandle<()>>) {
    let _ = child.start_kill();      // SIGKILL on Unix, TerminateProcess on Windows (tokio handles both)
    let _ = child.wait().await;      // reap, avoid Unix zombies
    for pump in io_pumps {
        pump.abort();                 // drop stdout/stderr reader tasks
    }
}
```

Forceful only in v1. MCP subprocesses use stdio (rmcp's `TokioChildProcess`), don't bind ports, are typically stateless. If a future MCP needs graceful shutdown, add Unix `SIGTERM` first via the `nix` crate (~25 lines). Not v1.

#### WS reconnect

MCP subprocesses **survive WS reconnect** — local lifecycle is independent of WS connectivity. On every fresh `hello_ack`:
1. Worker treats the new config as a fresh `config_update` and runs the diff-and-reconcile flow.
2. Worker **always** rebuilds and sends the `register_mcp` snapshot. The server's per-WS-session tools cache is invalidated when the WS session ended; we have to re-advertise on every reconnect, even if our local state is unchanged.

The "no `register_mcp` on Alive→Dead" optimization survives but only **within** a single WS session.

#### Three shared helpers

```rust
async fn spawn_mcp(config: &McpServerConfig) -> Result<(McpSession, McpSchemas), SpawnError>
async fn teardown_mcp(child: Child, io_pumps: Vec<JoinHandle<()>>)
fn build_register_mcp_frame(state: &McpMap) -> RegisterMcpFrame   // applies enabled filters
```

All three live in `plexus-client/src/mcp/`. The worker stitches them together for every lifecycle moment.

#### Explicit non-goals in v1

- **No auto-restart on crash.** Recovery is via `config_update` (user re-saves config).
- **No proactive system-prompt mention** that "MCP X is currently down". Agent learns by trying, gets diagnostic error.
- **No partial trickle registration.** One `register_mcp` per change-event keeps server cache invalidations bounded.
- **No graceful SIGTERM path.** Forceful kill only.
- **No cross-session parallelism** in tool dispatch. Worker queue is strict FIFO.
- **No retry on initial spawn timeout.** 30s once; failure → ADR-049 rejection path.

**Consequences:** Tight implementation (~150 LoC for the worker + helpers), zero generation counters, zero CAS dance, zero per-MCP locks. Race-condition surface area collapses to "subprocess crashes, the rmcp call returns an error, propagate normally per ADR-031" — which is just `Result<_, McpError>` propagation, not concurrency engineering. User-facing failure modes (collision, schema drift, spawn failure, filter change) all flow through the same server-orchestrated rejection path (ADR-049) and the same per-user SSE channel (ADR-106). Diagnostically useful via the structured `last_error` + reconfigure hint format.

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

### ADR-052 · `web_fetch` is shared; server hard-blocks private addresses, client default-blocks with per-device whitelist exceptions

**Status:** accepted
**Context:** `web_fetch` was originally server-only with a hardcoded private-IP block. With clients in the picture (and legitimate use cases like fetching an internal company API at `10.180.20.30:8080`), making `web_fetch` shared lets the agent reach declared internal services through the same structured tool path it uses for public URLs.
**Decision:** `web_fetch` is a shared tool. The merger's `plexus_device` enum = `["server"] + connected_clients`.

- **Server site:** unconditional block-list. RFC-1918 (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16), 100.64.0.0/10 carrier-grade NAT (covers Tailscale's 100.x range), 169.254.0.0/16 link-local, 127.0.0.0/8 loopback, IPv6 equivalents (`::1`, `fc00::/7`, `fe80::/10`). **No whitelist exception.** Protects neighbor infra in the same VPC/tailnet (e.g. another service on the same Tailnet that the agent must never be able to probe).
- **Client site:** same default block-list **plus per-device `ssrf_whitelist`** (host or `host:port` entries) overriding the block. Whitelist is editable per device (ADR-050).
- DNS rebinding mitigated at both sites: re-resolve before connecting, verify the actual connect-target IP against the policy.

**Capability declaration, not a sandbox.** The client whitelist is not a security boundary. The agent retains full host network access via `exec` (e.g. `curl 10.180.20.30:8080`) because `bwrap` does not isolate network (ADR-073). The whitelist exists to give the agent a clean, structured, audit-loggable tool path for declared internal services — not to prevent network access. The device-setup UI documents this so users aren't sold false security.

**Consequences:** Server stays hard-protected against neighbor-fetch attacks. Clients gain a declarative way to reach internal services through `web_fetch` without falling back to `exec curl`. Per-user SSRF whitelist is still gone (server is hardcoded); per-device whitelist is back as a capability declaration.

### ADR-096 · Device WebSocket protocol — single-connection JSON control + binary file transfer

**Status:** accepted
**Context:** Devices need bidirectional, low-latency dispatch (server pushes tool calls, client pushes results, both sides push file bytes for `message`-with-files and `file_transfer`). Browser already uses REST + SSE (ADR-003); devices need WebSocket because they sit behind NAT and tool dispatch is bidirectional.
**Decision:** A single WebSocket connection per device carries both control plane (JSON text frames) and bulk plane (binary frames). The full wire spec lives in `docs/PROTOCOL.md`; this ADR fixes the headline choices that other decisions reference:

- **Endpoint:** `GET /ws/device` with `Authorization: Bearer <PLEXUS_DEVICE_TOKEN>` (or `?token=` query for clients that can't set headers on WS).
- **Frame types (text/JSON):** `hello`, `hello_ack`, `tool_call`, `tool_result`, `register_mcp`, `config_update`, `transfer_begin`, `transfer_progress`, `transfer_end`, `ping`, `pong`, `error`.
- **Correlation:** every request carries a UUID v7 `id`; responses echo it. Not strict JSON-RPC.
- **Parallel tool dispatch.** Server may issue multiple `tool_call` frames before any `tool_result` arrives; client spawns a tokio task per call. Matches the agent's parallel-tool pattern.
- **Heartbeat.** Server sends `ping` every 30s. Two missed `pong` (~70s) → mark device offline, fail in-flight calls with `tool_result(is_error=true, code:device_unreachable)` (ADR-031). Client reconnects with exponential backoff using the same token; `hello` is idempotent.
- **No persistent in-flight queue.** Server does not retry on its own; if the client drops mid-call, the failure surfaces to the agent immediately. Agent decides next action.
- **File transfer (Option A).** Bulk bytes flow over the same WS as binary frames, multiplexed by a 16-byte UUID header per frame. JSON `transfer_begin` opens the slot (carries `total_bytes`, `sha256`, src/dst), `transfer_end` closes (verifies sha). Multiple transfers can be in flight concurrently. For device→device transfers, the server is a pure bridge — reads sender's binary frames, forwards to receiver's WS without buffering the whole file.
- **JSON for M0–M3.** MessagePack/CBOR is a future optimization; not justified for current scale.

**Consequences:** Client crate is a WS loop + local tool dispatcher + a binary-frame multiplexer. No HTTP listener required (clients can be behind any NAT). All device-related ADRs (config push ADR-050, MCP register ADR-047, tool call ADR-031, transfer ADR-087) hang off this protocol.

### ADR-097 · Device pairing — frontend-issued token, env-var startup, token-as-identity

**Status:** accepted
**Context:** Devices need to identify themselves to the server. Must work for headless boxes (`./plexus_client` on a server), unattended phones, and dev laptops. No browser-side OAuth dance.
**Decision:** Pairing is a one-shot token-issuance flow:

1. **Token creation** (frontend, web UI). User opens "Devices" page, fills in `name`, optional `workspace_path`/`fs_policy`/`shell_timeout_max`/`ssrf_whitelist`/`mcp_servers`, submits.
2. **Server mints token.** `POST /api/devices` returns `{token: "plexus_dev_<base64>", ...}` ONCE. Token is shown verbatim in the UI with copy-to-clipboard. Never retrievable again — lost tokens require `POST /api/devices/{name}/regenerate-token` (ADR-091).
3. **Client startup.** User exports `PLEXUS_DEVICE_TOKEN=plexus_dev_...` and runs `./plexus_client` (or whatever the installed binary is called). Token is the **only** identifier the client needs; everything else (workspace path, fs_policy, etc.) is fetched from the server's `hello_ack` frame at handshake.
4. **Identity.** The token is the SSOT for device identity — primary key on `devices` (ADR-091). `(user_id, name)` UNIQUE means a user can't have two devices with the same friendly label, but the friendly label is purely cosmetic; the token is what identifies the connection.
5. **Rotation.** Delete + recreate (frontend) or `POST /api/devices/{name}/regenerate-token`. Old token invalid immediately; in-flight WS connection torn down on next server-side check.

**Consequences:** No QR codes, no out-of-band pairing dance, no browser launching from the client. Headless deployments are trivial (`export PLEXUS_DEVICE_TOKEN=...`). Token leaks are equivalent to device compromise — same blast radius as exposing any bearer credential; user rotates and moves on. ADR-073's config-masking covers the disk-side leak vector for the client binary's local config.

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
**Decision:** Cascades defined at table-create time, not via `ALTER TABLE` migrations. Account deletion is a single `DELETE FROM users WHERE id = $1` that cleans up devices (tokens are inline per ADR-091), sessions, messages, cron_jobs, discord_configs, telegram_configs automatically.

### ADR-059 · Messages store provider-shape content blocks as JSONB; images inline as base64 data URLs

**Status:** accepted
**Decision:** `messages.content JSONB` holds the array of content blocks. Block shapes mirror the OpenAI chat-completions request body exactly (ADR-101 — Plexus speaks OpenAI chat completions only, gateway-translated for non-OpenAI providers) — storing what the LLM will receive so the request body is a pass-through with no translation.

Canonical block types:
- **text:** `{"type": "text", "text": "..."}`
- **image:** `{"type": "image_url", "image_url": {"url": "data:image/png;base64,..."}}` — bytes inline as base64 data URL, not a path. The workspace copy at `.attachments/` (ADR-044) is for the agent's file tools; the DB copy is for durable conversation replay.
- **tool_use / tool_result:** per the provider's tool spec.

User, assistant, and tool messages all use the same block schema.

**Consequences:**
- No translation between DB storage and LLM request — content blocks go out to the provider as-stored.
- History replay is full-fidelity for VLMs and for non-VLMs (vision-retry strips `image_url` blocks per ADR-026, leaving path-text markers per ADR-027).
- Frontend can render images by emitting the base64 data URL directly — no extra fetch.
- Durable if the workspace attachment is deleted: conversation history continues to render correctly even when the workspace copy is gone (user-initiated cleanup, per ADR-044 + ADR-081).
- DB rows with images can be large (MBs). Message storage grows with images independently of workspace quota. For v1 this is acceptable; if DB bloat becomes a real concern, a future optimization is to strip `image_url` blocks from compacted rows (keep text/path markers) — tracked as a future enhancement, not a v1 constraint.

### ADR-060 · No `users.soul`, `users.memory_text`, `users.ssrf_whitelist`

**Status:** accepted
**Decision:** SOUL.md and MEMORY.md are files in the user's workspace, not DB columns. Per-user SSRF whitelist doesn't exist server-side (ADR-052); only per-device whitelists.
**Consequences:** Editable by the agent via file tools without specialty endpoints. Inspectable on disk. Versioned naturally via git if the user cares.

### ADR-089 · Message role enum is `user | assistant | tool`; compaction summaries use an inline flag

**Status:** accepted
**Context:** Compaction summaries (ADR-028) need to be distinguishable from regular history so the next compaction pass skips them and the context builder knows where the "fold point" is. A synthetic role like `compaction_summary` would break the pass-through-to-provider storage (ADR-059), since OpenAI chat completions (the only API Plexus speaks per ADR-101) only accepts `user`, `assistant`, `tool`.
**Decision:**
- `messages.role` column is strictly one of `user`, `assistant`, `tool`. No synthetic role values.
- Compaction summaries are inserted with `role='assistant'` plus `is_compaction_summary BOOLEAN NOT NULL DEFAULT FALSE` set to true on that specific row.
- **Context builder:** loads the most recent row where `is_compaction_summary=true` (if any), then every message newer than it. Pre-summary rows are not loaded but remain in DB for audit.
- **Compaction pass:** skips rows where `is_compaction_summary=true` so a summary never gets re-summarized.
**Consequences:** Content JSONB is pass-through to the provider — the summary appears as a regular assistant message in the LLM request. The flag is a purely internal marker, never serialized outside DB. No special provider-side handling.

### ADR-090 · Per-channel bot configs live in their own tables

**Status:** accepted
**Context:** Discord, Telegram, and any future messaging channel each carry several fields (bot token, partner chat identifier, channel-specific flags). Inlining these as columns on `users` is feasible but bloats the users row, couples unrelated fields together, and has to change every time a new channel is added.
**Decision:** Each connected-channel type owns its own table: `discord_configs`, `telegram_configs`, etc. Each has `user_id` as FK (ON DELETE CASCADE per ADR-058), the channel's `bot_token`, a partner-identifier field (`partner_chat_id`), and whatever channel-specific settings the integration needs. Users table stays thin — no inline channel fields.
**Consequences:** Adding a new channel = adding a new table, no users-schema change, no migration pressure on unrelated features. Channel config is naturally scoped: a user with Discord configured but no Telegram has a row in `discord_configs` and none in `telegram_configs`. Account deletion cascades to all channel tables automatically.

### ADR-091 · Device identity: `token` is PK, `(user_id, name)` is UNIQUE, user-initiated regenerate only

**Status:** accepted
**Context:** Devices need an internal identifier (for handshake auth + row identity) and an external reference (for URLs, tool routing, system-prompt device enum). Early idea was "device id = device token" so only one field exists. But if the token is the identifier, `PATCH /api/devices/{token}/config` embeds the auth secret in URL paths, which end up in access logs, reverse-proxy traces, browser history, and debugging tools. That's a token-leak hazard even for self-hosted deployments.
**Decision:**
- **`devices.token`** — random secret. Primary key of the row. Acts as the canonical internal device identifier (for direct lookups, future FK references, etc.). Stored in plaintext (it IS the credential, not a credential wrapper).
- **`devices.name`** — user-assigned friendly label ("laptop", "desktop"). Required. UNIQUE within a user via a `UNIQUE (user_id, name)` constraint.
- **Handshake auth:** client sends `Authorization: Bearer <token>` on WebSocket connect. Server looks up the device by `token` (primary key). If found and not banned, connection proceeds.
- **REST admin endpoints:** use the friendly name. `PATCH /api/devices/{name}/config`, `DELETE /api/devices/{name}`, `GET /api/devices/{name}`. JWT supplies user_id; server looks up by `(user_id, name)`. Token never appears in URLs.
- **Agent tool calls:** the `device` argument uses the friendly name ("laptop"). Server routes by `(session.user_id, name)` lookup. Token stays invisible to the agent.
- **No automatic token rotation.** User triggers regenerate explicitly from the settings UI ("regenerate token" button). Regenerate overwrites the `token` column, disconnects the currently-connected device (handshake auth will no longer find the old token), and displays the new token to the user once. The user pastes the new value into the client config. No mid-job expiration, no rotation scheduler.

**Consequences:**
- Tokens never appear in URLs, logs, or any agent-visible surface.
- Two users can both name a device "laptop" — the scoping via `user_id` keeps names friendly without collision risk.
- One row per device. No separate `device_tokens` table.
- Regenerate is the user's explicit action; we never surprise them with token changes.
- A lost/leaked token is fixed by pressing regenerate, not by opaque rotation machinery.

### ADR-092 · No heartbeat state is persisted

**Status:** accepted
**Context:** Heartbeat Phase 1 (ADR-054) runs each tick and decides skip-or-run based on current time and `HEARTBEAT.md`. A "last Phase 1 decision" column or table was considered to let admins audit tick behavior.
**Decision:** No persisted heartbeat state. No `users.last_heartbeat_phase1_at`, no `heartbeat_state` table. Phase 1 is stateless — each tick reads current context and decides fresh.
**Consequences:** Restart doesn't carry heartbeat baggage. If Phase 1 fires Phase 2, the only persistence is the resulting heartbeat-session message history (via the normal message-bus path, ADR-010). Admin audit of Phase 1 behavior must come from logs, not DB queries. Acceptable: heartbeats are infrequent and user-scoped, not a compliance surface.

### ADR-093 · Chat SSE stream unifies history replay + live events

**Status:** accepted
**Context:** Original shape (ADR-003) used two endpoints for browser chat: `GET /api/sessions/{id}/messages` for paginated history and `GET /api/sessions/{id}/stream` for live SSE. On chat open the frontend had to call both — a GET for history, then open the stream for live — and handle the race where a message could arrive between the two requests. Client-side deduplication by `message_id` papers over it but is extra complexity for every chat consumer.
**Decision:** The SSE stream at `GET /api/sessions/{id}/stream` is the canonical "show me the chat" endpoint. On connect:

1. **Replay phase.** Server emits the most-recent persisted messages as `event: message` chunks, chronologically ordered, capped by the `replay_limit` query param (default 50, max 200). If the request carries `Last-Event-ID`, replay is "everything after that message_id" — EventSource's native reconnect seamlessly fills missed events.
2. **Cut-over.** Server emits one `event: history_end` marker.
3. **Live phase.** Server emits `event: hint`, `event: message`, `event: session_update`, `event: kick` as they occur.

Each `event: message` carries an `id:` SSE header with the DB `message_id`, so the browser's EventSource can reconnect with `Last-Event-ID` and the server replays exactly what was missed.

The `GET /api/sessions/{id}/messages` endpoint stays but narrows in purpose: it is now the **cursor-paginated scroll-up** entry point (`?before=<msg_id>&limit=50`), used only when the user scrolls past the window the stream replayed.

**Consequences:**
- One API call on chat open instead of two. Frontend code drops a race condition and a deduplication pass.
- Native reconnect via `Last-Event-ID` means dropped connections replay exactly the missed events — no per-message dedup on the client.
- Replay payload at 50 messages is bounded (base64 images included — still a few hundred KB typical). Older batches come through the messages endpoint.
- The original `final` event type dissolves: a terminal assistant message is just a persisted `message` event in the unified model. Transient events (`hint`) remain distinct because they're not persisted.
- Multi-tab: opening a second tab replays the same history to that subscriber, live events broadcast to all SSE subscribers for the session. No extra coordination needed.

### ADR-106 · Per-user SSE event channel for account-scoped notifications

**Status:** accepted
**Context:** ADR-093's chat SSE stream is **per-session** (`GET /api/sessions/{id}/stream`). Some events are **per-user** — they're not tied to any particular chat session: an MCP install was rejected on a device (ADR-049), a quota threshold was crossed, an LLM provider config validation failed, a device went offline. These don't fit cleanly into a per-session stream because the user might not have any chat session open when they fire, and they apply across all the user's surfaces (devices, channels, configs).
**Decision:** A second SSE endpoint, `GET /api/me/events`, scoped to the authenticated user. Frontend opens it once per browser session (in addition to whatever per-session chat streams it has open) and keeps it alive for the duration of the page lifetime.

Event types are user-scoped and asynchronous to any chat session:

| Event | Payload | Triggered by |
|---|---|---|
| `mcp_rejected` | `{ device, server, reason: "schema_collision" \| "within_server_collision" \| "spawn_failed", detail }` | ADR-049 (collision on register) or ADR-105 (spawn failure on client) |
| `device_offline` | `{ device, last_seen_at }` | WS heartbeat timeout (PROTOCOL.md §1.4) |
| `device_online` | `{ device }` | WS handshake completed for a previously-offline device |
| (future) `quota_warning` | `{ used_bytes, quota_bytes }` | Per-user quota crosses 90% threshold |
| (future) `llm_config_invalid` | `{ key, error }` | LLM provider config validation failed at startup or after admin edit |

Reconnect via `Last-Event-ID` using a per-user `event_id` cursor (UUIDs minted at emit time, persisted briefly server-side or replayed-from-memory if recent). Users opening the page after a long absence get nothing replayed — events are best-effort and ephemeral; if the user wasn't connected when one fired, they get the result via the normal data fetch (e.g. opening Settings → Devices shows the rejected device gone from the list).

Auth: same JWT cookie as everything else (ADR-004). EventSource sends cookies natively.

**Consequences:** Frontend has TWO SSE endpoints open: per-session chat stream(s) + one per-user event stream. Chat events stay where they are (session-scoped). Account-scoped notifications get a clean separate channel that doesn't pollute the chat protocol. Adding new account-scoped event types is additive — no breaking changes to chat. The endpoint is also useful for future features like live device-status indicators in the Devices tab and in-app notifications.

### ADR-094 · Runtime block is persisted per user message as historical metadata

**Status:** accepted
**Context:** Each inbound user message carries a small `<runtime>` block with time, channel, and chat_id (per SYSTEM_PROMPT.md). Earlier wording left it ambiguous whether this block is part of the persisted message or injected fresh per LLM call. Codex flagged the risk of stale timestamps leaking from old history. The concern dissolves if we treat runtime blocks as timestamped historical metadata — each old runtime block correctly records *when that message arrived*, not "current state."
**Decision:**
- The `<runtime>` block is constructed **once**, at user-message ingress time (in the channel adapter or `publish_inbound` path), with then-current time + channel + chat_id.
- It is prepended to the user's content blocks inside the same `messages.content` JSONB row (per ADR-059), as a text block.
- It is **immutable** after insert. No later regeneration, no stripping on replay.
- On history read, the agent sees a chronologically ordered sequence of user messages, each with its own runtime block labeling when it arrived. The most-recent one describes "now"; older ones describe the past.

**Consequences:**
- Agent naturally understands temporal flow: *"user asked at 10:00, now it's 17:00, they're asking a follow-up"*. Old blocks aren't confusion — they're context.
- No fresh-injection step per LLM call. Persisted state is the LLM's state.
- Cache-friendly: a session's history grows by append only; the system prompt + prior history are stable for prompt caching, only the new user message (including its freshly-constructed runtime block) is novel per turn.
- Multi-iteration turns (tool use loops): the runtime block was set at message arrival; across iterations inside one turn, it stays the same. "Now" only advances when a new user message arrives.

### ADR-095 · All tool results are prefixed with `[untrusted tool result]: ` at construction time

**Status:** accepted
**Context:** Tool-returned content (web_fetch bodies, shell stdout, MCP responses, even `read_file` output from files of unknown provenance) can carry instructions crafted to hijack the agent. Channel inbound content is already marked untrusted via the `[untrusted message from <name>]:` wrap (ADR-007). Tool output had no analogous structural marker. Codex flagged this as a prompt-injection vector.
**Decision:** Every `tool_result` content is prefixed with the literal string `[untrusted tool result]: ` at construction time, uniformly across all tools (shared, server-only, client-only, MCP). A shared helper in `plexus-common/src/tools/result.rs` wraps the content before the dispatcher emits the `tool_result` block.

The wrapped shape the LLM sees:

```
{
  "type": "tool_result",
  "tool_use_id": "toolu_xyz",
  "content": "[untrusted tool result]: <raw bytes the tool returned>"
}
```

No system-prompt rule is added. The wrap itself is the signal — the agent learns the convention structurally, the same way it learned the `[untrusted message from X]:` channel wrap (ADR-007). No teaching, no exception rules, no provenance arguments.

**Consequences:**
- Prompt-injection defense becomes uniform across all untrusted content: channel messages AND tool outputs both arrive structurally wrapped.
- One codepath wraps everything — no per-tool opt-in, no forgotten tool with raw content.
- The agent can still *use* information inside tool results; it just doesn't follow instructions embedded there. Same distinction as for channel messages.
- Compaction, persistence, and LLM-call pass-through all work unchanged — the wrap is just part of the content string.

### ADR-098 · REST `/api/sessions/{key}/messages` accepts only frontend session keys; reads are open

**Status:** accepted
**Context:** Session keys follow `{channel}:{chat_id}` or an override (ADR-006). Internal synthesizers use overrides like `cron:{job_id}` and `heartbeat:{user_id}`. A user with valid auth could otherwise call `POST /api/sessions/cron:abc/messages` and inject a synthetic-looking message into a cron job's history, or `POST /api/sessions/discord:9999/messages` to forge a Discord-origin message. Codex-style audit flagged this as a session-key injection vector.
**Decision:** Asymmetric per verb on the session-key path:

- **Write** — `POST /api/sessions/{key}/messages`: **allow-list** the channel prefix. Only keys whose prefix is `web:` (the frontend's namespace) are accepted. Everything else (`cron:`, `heartbeat:`, `discord:`, `telegram:`, any future channel) is rejected with `400 Bad Request`. Default deny — no block-list to maintain. The frontend can only inject into its own `web:` sessions; Discord/Telegram/cron/heartbeat sessions get their messages exclusively from their own pipelines (channel adapters, scheduler, heartbeat phase-2 synthesizer).
- **Read** — `GET /api/sessions/{key}/stream`, `GET /api/sessions/{key}/messages`, `GET /api/sessions/{key}`: any key whose `session.user_id` matches the authenticated user. The web UI can show Discord history, cron history, heartbeat history — read-only — without being able to write into those streams. Impersonation isn't possible through a read.
- The user-ownership check (`session.user_id == jwt.user_id`) applies to both verbs unchanged.

**Consequences:** Frontend retains a clean inbox into its own web-channel sessions. Internal session namespaces stay sealed against impersonation. The web UI can render any of the user's session histories (the multi-tab "show me what my agent said on Discord today" view) without exposing a forge primitive. No block-list bookkeeping — adding a new channel namespace later doesn't require remembering to deny-list it.

---

## 10. Safety

### ADR-072 · Server is not a code execution environment for agents

**Status:** accepted
**Context:** The server hosts user workspaces on disk — SOUL.md, MEMORY.md, `skills/`, `.attachments/`, arbitrary user-uploaded files. Any of these could contain executable content (a shell script, a Python file, a binary). The agent itself can write such content via `write_file`. The question: can the agent, or the content, cause the server to execute something?
**Decision:** **No.** The agent's server-side tool surface is deliberately restricted to non-executing operations:

- **File tools** (`read_file`, `write_file`, `edit_file`, `delete_file`, `list_dir`, `glob`, `grep`) — byte-level operations through `workspace_fs`. Read and write content, never interpret it.
- **`message`** — delivers text/media to a channel. No execution.
- **`web_fetch`** — HTTP GET/POST. When dispatched to the server site, the unconditional block-list (RFC-1918, 100.64/10, link-local, loopback, IPv6 equivalents — ADR-052) applies. Content is returned as bytes; server does not evaluate.
- **`cron`** — schedules future agent invocations. Does not itself execute anything.
- **`file_transfer`** — moves bytes between server and a device. No execution.

Absent, deliberately: `exec`, `python`, `eval`, any code-execution tool (on the SERVER — `exec` is a CLIENT-only tool).

**Consequence:** An agent that writes `rm -rf /` into `~/workspace/evil.sh` cannot trigger its execution on the server. Same for anything in MEMORY.md, SOUL.md, `skills/*/SKILL.md`, `.attachments/`. The server treats all user/agent-provided files as inert data.

**Corollary — server-side MCP subprocesses are the one admin-gated exception.** Admin-installed MCPs (ADR-047) run as `TokioChildProcess` via rmcp. This is intentional code execution, but access is:
- Admin-configured only (`PUT /api/server-mcp`, admin JWT required).
- Not agent-reachable beyond the MCP's declared tool schemas.
- Schema-collision-checked at install (ADR-049).

Admin is trusted. Agent is not. The shape of "admin explicitly installs; agent calls tools through protocol" keeps the blast radius bounded to what the MCP itself exposes.

### ADR-073 · Client sandboxing — two distinct jails: file-tool jail (in-process, OS-agnostic) + subprocess jail (out-of-process, OS-specific)

**Status:** accepted
**Context:** The client's purpose is the opposite of the server's — it exists to give the agent code-execution capability on the user's device (shell commands, MCP subprocesses, file writes that users will then run themselves). That necessarily creates a risk surface. Plexus runs on Linux (dev-ops boxes), macOS (laptops), and Windows (engineer workstations); a single sandbox primitive doesn't span all three. Nanobot's lesson: file-tool path validation in code is OS-agnostic and load-bearing; bwrap is additional protection for subprocesses on Linux only.
**Decision:** **Two distinct jail mechanisms**, both controlled by `fs_policy`:

#### Jail 1 — file-tool jail (in-process, Rust path validation, OS-agnostic)

Every file tool implemented in `plexus-client` (`read_file`, `write_file`, `edit_file`, `delete_file`, `delete_folder`, `list_dir`, `glob`, `grep`, `notebook_edit`) calls a shared helper — `resolve_in_workspace(path: &str) -> Result<PathBuf>` in `plexus-common/src/tools/path.rs` — before any disk operation. The helper:

1. Expands `~` and resolves relative paths against `device.workspace_path`.
2. Calls `Path::canonicalize()` to dereference symlinks.
3. Verifies the resolved absolute path is `starts_with(device.workspace_path.canonicalize())`.
4. Rejects with `WorkspaceError::PathOutsideWorkspace` otherwise.

This is **mandatory on every platform** — Linux, macOS, Windows — because it's pure Rust, no OS primitive required. The agent's file tools cannot escape `workspace_path` regardless of OS. Hard guarantee. Matches nanobot's `_resolve_path()` pattern (`nanobot/agent/tools/filesystem.py:17–33`).

#### Jail 2 — subprocess jail (out-of-process, OS-specific)

`exec` and client-side MCP subprocesses spawn child processes. Path validation in Rust doesn't help once the subprocess is running — it can do whatever the host lets it. So this jail is OS-dependent:

| OS | v1 mechanism | What it does | What's deferred |
|---|---|---|---|
| **Linux** | `bwrap` jail rooted at `workspace_path` | Filesystem-only isolation; network open. Mount list matches nanobot exactly (see below). | — |
| **macOS** | none | Subprocess runs with full user privileges. Env-stripped (see below). | `sandbox-exec` profile (Apple-deprecated but still used by Claude Code, Cursor; ~200 LoC + macOS testing). |
| **Windows** | none | Subprocess runs with full user privileges. Env-stripped. | AppContainer (multi-week scope) or Windows Sandbox (heavy install). |

**Linux bwrap mount list** (`--ro-bind-try`, matches nanobot exactly):
- `/usr`, `/bin`, `/lib`, `/lib64` — binaries + dynamic linker.
- `/etc/alternatives` — Debian alternatives symlinks.
- `/etc/ssl/certs` — TLS root certificates.
- `/etc/resolv.conf` — DNS resolution (without this, `curl example.com` fails).
- `/etc/ld.so.cache` — dynamic linker cache.

`workspace_path` is bind-mounted read-write at its same absolute path. Everything else is `tmpfs`. Host env stripped to `PATH`, `HOME`, `LANG`, `TERM`.

#### `fs_policy` controls both jails together

| `fs_policy` | File-tool jail | Subprocess jail |
|---|---|---|
| `"sandbox"` (default) | enforced (all OSes) | Linux: bwrap. macOS/Windows: env-stripped only. |
| `"unrestricted"` | **lifted** — agent's file tools can read/write anywhere on the host | Linux: no bwrap. macOS/Windows: env-stripped only. |

Matches nanobot's behavior (`nanobot/agent/loop.py:286–288` — `allowed_dir = workspace if (restrict_to_workspace or sandbox) else None`). `unrestricted` is a coherent "this is my dev box, agent has my privileges" mode.

Toggling to `unrestricted` requires typed-device-name confirmation (ADR-051).

#### Network is NOT isolated (Linux bwrap)

`bwrap` is filesystem-only in Plexus — we deliberately do not pass `--unshare-net`. The agent retains full host network access. Intentional: agents that can't run `pip install`, `npm install`, or `curl` aren't useful as coding agents. The sandbox prevents exfiltration of host files (e.g. `~/.ssh/id_rsa`), not network probes. Network-egress controls live in `web_fetch`'s per-device whitelist (ADR-052), explicitly framed as capability declarations, not enforced boundaries — `exec curl` always bypasses them.

#### Environment stripping (all OSes, even unrestricted)

Even in `unrestricted` mode, the env passed to subprocesses is stripped to a small allow-list before exec:
- **Linux/macOS:** `PATH`, `HOME`, `LANG`, `TERM`.
- **Windows:** `SYSTEMROOT`, `COMSPEC`, `USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `TEMP`, `TMP`, `PATHEXT`, `PATH`, `APPDATA`, `LOCALAPPDATA`, `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`, `ProgramW6432` (matches nanobot, `tests/tools/test_exec_platform.py:65–73`).

Secrets in `$GITHUB_TOKEN`, `$AWS_SECRET_ACCESS_KEY`, etc. don't leak to agent-run processes regardless of `fs_policy` or OS.

#### Client config masking (Linux only)

The client process stores its device token at `~/.config/plexus/client.yaml` (or `$XDG_CONFIG_HOME/plexus/client.yaml`). The Linux bwrap jail `tmpfs`-masks the parent of that path so even an `fs_policy=sandbox` agent's subprocesses can't read it. Client startup refuses any `workspace_path` that overlaps the config directory on every platform (validation error → process exits before any session opens), so even on macOS/Windows where the subprocess jail doesn't exist, the file-tool jail + workspace boundary together keep the config out of reach.

**Consequences:**
- **Cross-platform sandbox is real, just narrower.** All three OSes get the file-tool jail; only Linux gets the subprocess jail in v1. Sandbox mode means "the agent's file tools cannot escape the workspace; on Linux, exec/MCP subprocesses are also jailed." Honest framing in the device-setup UI.
- **The sandbox is a defense, not a guarantee.** `bwrap` namespace isolation is strong but not unbreakable; macOS/Windows sandbox modes are weaker by design. Plexus documents the risk in the device setup UI.
- **Future sandbox primitives slot in via `fs_policy` mechanism.** Adding macOS support means shipping a `sandbox-exec` profile + wrapper code; Windows means tackling AppContainer. No protocol change.
- **`unrestricted` is the same name on every OS** — coherent semantics: file tools can roam, subprocesses run with full host privileges (env still stripped).

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
- **Quota DoS via noisy allowed-users.** If an allowed user (a non-partner human the partner has authorized to message the agent on a shared channel — e.g. a coworker added for after-hours ops) spams files or messages and burns the partner's storage / LLM quota, mitigation is the partner removing them from their per-channel allow-list. Not a platform-level concern.

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
**Status:** superseded by ADR-087. Originally "single-file ops only; delete + re-upload for rename." Rename/move (including folder rename) is now supported via `file_transfer` with `mode=move` — same-device move is an atomic `tokio::fs::rename`. Bulk operations remain out of scope.

### ADR-068 · No server-pushed workspace tree invalidation
When an agent writes a file, the open Workspace tab doesn't auto-refresh. User reload or navigate triggers refetch. WS/SSE push can be added if the UX friction is real.

### ADR-069 · No real migrations framework in v1
`include_str!("schema.sql")` with `IF NOT EXISTS` semantics is all. Add `sqlx::migrate!` when first real user arrives.

### ADR-070 · No multi-instance-coordination for heartbeat
Heartbeat tick runs per-process. If two servers run the same DB, both would fire heartbeats. Single-node deployment avoids this. Coordinating across nodes requires leader election or advisory locks — deferred.

### ADR-103 · No multi-server multiplexing in plexus-client
One client process talks to exactly one Plexus server. `PLEXUS_DEVICE_TOKEN` is a single value, the WS connection is a single endpoint, all in-memory state (config from `hello_ack`, in-flight tool calls, MCP sessions) is single-server. Users who need to participate in multiple Plexus deployments run the binary twice with different env vars. Adds no extra plumbing — separate processes are already isolated by OS.

---

## 12. LLM Provider

### ADR-101 · OpenAI chat completions API only; LLM config is admin-API-set, not env

**Status:** accepted
**Context:** Plexus needs an LLM. The choices: (a) ship a per-provider client trait (Anthropic Messages API, OpenAI Chat Completions, Bedrock, Gemini, etc. — each with its own request/response/tool-call shape), (b) speak one wire format and let the admin put a translation gateway in front for everything else. Option (a) has been the prior-Plexus pattern and produced provider-switching bugs, vision-strip drift, and tool-call-format edge cases.
**Decision:** **OpenAI chat completions API ONLY.** Plexus speaks one request shape, one response shape, one tool-call format. If an admin wants Anthropic / Bedrock / Gemini / a local model, they put a gateway in front (LiteLLM, an OpenAI-compatible proxy, or the provider's own OpenAI-compat endpoint) and configure Plexus to talk to it. Format translation lives in the gateway, not in Plexus.

The admin configures the LLM via the admin REST API — **not env vars**. Five keys persist in `system_config`:

| Key | Type | Purpose |
|---|---|---|
| `llm_endpoint` | string | Base URL of the OpenAI-compatible API (e.g. `https://api.openai.com/v1`, `http://litellm:4000/v1`). |
| `llm_api_key` | string | Bearer credential the server uses on outbound requests. |
| `llm_model` | string | Model name passed in the request body (e.g. `gpt-4o`, `gpt-5-codex`, `anthropic/claude-opus-4-7` if the gateway routes it). |
| `llm_max_context_tokens` | integer | The LLM's hard context-window size in tokens (e.g. `128000` for gpt-4o, `200000` for gpt-5-class). Counted with `tiktoken-rs` (ADR-025) against the full chat-completions prompt — system + tools + history + new turn. |
| `llm_compaction_threshold_tokens` | integer | Headroom that triggers compaction (default `16000`, ADR-028). When `llm_max_context_tokens − tiktoken_count(prompt) < llm_compaction_threshold_tokens`, the bus fires stage-1 compaction. The summary's `max_output_tokens` is `threshold − 4000` (= `12000` at the default), reserving 4k headroom for the next user turn. |

Set via `PATCH /api/admin/config`. Read via `GET /api/admin/config`. No `LLM_*` env vars; the only env vars relevant to LLM behavior are `DATABASE_URL` (so the server can read these keys at startup) and the JWT/auth secrets.

**Consequences:** No provider abstraction trait, no per-provider modules, no vision-format adapters per provider — vision retry (ADR-026) targets a single request shape. Switching the model is a `PATCH` away. Switching the *provider* is "stand up LiteLLM, change `llm_endpoint` and `llm_api_key`" — handled outside Plexus. Admin operating overhead (one extra container if they want non-OpenAI) is the trade we're willing to make for codebase simplicity.

---

## 13. Distribution

### ADR-102 · Distribution targets — Linux-only server (musl), all-three-OS client; GitHub Releases as the sole channel

**Status:** accepted
**Context:** Plexus serves a heterogeneous user base — Linux dev-ops boxes, macOS leadership, Windows engineers — but the production server is overwhelmingly Linux. We need a release strategy that ships single-binary artifacts for the realistic deployment matrix without taking on distro-packaging or container-distribution burden.
**Decision:**

**Targets:**

| Crate | Targets | Linkage |
|---|---|---|
| **plexus-server** | `linux-x86_64`, `linux-aarch64` | musl static |
| **plexus-client** | `linux-x86_64`, `linux-aarch64`, `darwin-x86_64`, `darwin-aarch64`, `windows-x86_64.exe` | musl on Linux; native libc on macOS/Windows |

The server's macOS/Windows targets are deliberately omitted in v1 — production deployment is overwhelmingly Linux, and supporting Windows server adds non-trivial code complexity (UNC path normalization for `messages.content` path-text markers, Windows symlink + junction handling in `workspace_fs` per ADR-045, ACL semantics for `skills/` validation). Admins who want to run the server on macOS/Windows can `cargo build --release` and accept untested status. Revisit post-M3 if real demand emerges.

**Linux uses musl.** All Plexus dependencies are pure Rust (sqlx, rustls, axum, tungstenite, rmcp), so musl-static linking produces one binary per architecture that runs on every distro from ancient CentOS to current Alpine without modification. No need for Debian/CentOS/RHEL-specific builds. Trade-offs (slower musl malloc, historically funky DNS resolver) are negligible for a network-bound service.

**Naming:** `plexus-{server,client}-v{X.Y.Z}-{os}-{arch}[.exe]`. Server tarball includes the embedded frontend bundle (per ADR-002). Client is a single static binary.

**Channel:** **GitHub Releases only**, tagged per version. No Docker images in v1 (revisit when there's first-real-deployment demand). No APT/YUM repos. No Homebrew tap. `cargo install --git github.com/<owner>/plexus` works as a fallback for users who already have a Rust toolchain and want to track main.

**M3 frontend integration:** the **Settings → Devices** tab surfaces a download link section. Frontend reads the deployed server's `GET /api/version` and renders direct links to the GitHub Release assets pinned to that exact version (so a deployment running v0.3.4 doesn't push users a v0.4.0 client that may not handshake against the older protocol). User-agent detection picks the matching binary as the primary CTA; the other targets sit behind a "Other platforms" disclosure.

**Consequences:** One channel to maintain (GitHub Releases). One binary per (crate × target). Linux distro-independence comes for free via musl. Frontend's download UX is version-correct by construction. Future container/distro-package channels add zero ADR debt because GitHub Releases is just "the artifact store" — anything else is a republishing layer over it.

### ADR-104 · plexus-client CLI surface, env vars, and failure semantics

**Status:** accepted
**Context:** plexus-client is a long-running daemon-style process invoked by the user (or systemd / launchd / Windows service / `nohup ./plexus-client &`). It needs the smallest possible startup contract — env vars in, no config wizard, no flags for the common path. Failure modes also need clear conventions so users on three OSes know what "broken" looks like.
**Decision:**

#### Env vars (both required for `run`)

| Var | Example | Purpose |
|---|---|---|
| `PLEXUS_DEVICE_TOKEN` | `plexus_dev_abc123...` | Device identity + auth (ADR-091, ADR-097). Created by the user via `POST /api/devices`, shown once in the frontend. |
| `PLEXUS_SERVER_URL` | `https://company.plexus.com` (prod) or `http://localhost:8080` (dev) | Base URL with scheme. Client derives the WS endpoint by swapping `http(s)` → `ws(s)` and appending `/ws/device`. No path component supported in v1 (server is at the URL root; deployments behind path-prefix proxies are out of scope). |

Missing or empty env var → friendly stderr message + exit non-zero.

#### CLI subcommands

```
plexus-client run           # default subcommand if invoked with no args
plexus-client version       # print "plexus-client v0.X.Y (protocol v1)" and exit
plexus-client logout        # deregister this device server-side, then clear local config
```

No other subcommands in v1. No `doctor` (failure modes self-explain), no `status` (use the web UI's Devices tab), no `--config` flag (env vars carry everything).

#### `logout` semantics

`logout` is **local-only**. It removes `~/.config/plexus/` (or `$XDG_CONFIG_HOME/plexus/`) and exits zero. The device token remains valid server-side — full revocation is the user's action via the frontend Devices tab (`DELETE /api/devices/{name}`). The CLI prints:

```
Logged out locally. The device token is still valid server-side.
Revoke it via Settings → Devices on the web UI to fully deregister.
```

No new API endpoint required. The client never makes a REST call during normal operation — everything goes over WS. Server-side deregister is intentionally a frontend action so that "I lost my laptop" still works (user revokes from any browser, no need to recover the missing device first).

#### Sandbox fallback (Linux + `fs_policy=sandbox` + bwrap missing)

**Silent fallback to env-stripped subprocess execution.** If the device's `fs_policy=sandbox` but `bwrap` isn't on the host's `PATH`, the client logs a warning at startup ("`bwrap` not found; subprocesses will run env-stripped without filesystem isolation. Install with `apt install bubblewrap` or set fs_policy=unrestricted to silence this warning.") and proceeds. File-tool jail (ADR-073) is unaffected — it's pure Rust path validation, no OS primitive needed. This matches nanobot's behavior on macOS/Windows extended to Linux-without-bwrap.

#### Initial connect retry — backoff forever

Client never gives up reaching the server. On startup, if the WS handshake fails (DNS error, TCP refused, TLS error, 4xx response, etc.):

- Retry with the same exponential backoff used post-handshake (PROTOCOL.md §1.3): 1s, 2s, 4s, 8s, 16s, 30s, 30s, ..., capped at 30s with ±20% jitter.
- Log each attempt to stderr.
- Never exit on its own; only SIGTERM / SIGINT / OS shutdown stops it.

Rationale: the typical deployment is `systemd Restart=always` or equivalent, so the daemon should be self-healing rather than die-and-be-restarted. For interactive debugging the user can `Ctrl-C`. No `--exit-on-error` flag in v1; add later if a real use case appears.

#### Local config dir contents (v1)

`~/.config/plexus/` exists primarily so the Linux bwrap jail can `tmpfs`-mask it (ADR-073). In v1 the directory is **empty** — env vars carry all state, every WS reconnect re-fetches config via `hello_ack`, no local cache, no log file (logs go to stderr; user redirects via shell or systemd journal). Future versions may cache the last `hello_ack` for faster startup; for now, simplicity wins.

#### Workspace directory bootstrap

When `hello_ack` arrives carrying `workspace_path`, the client:

1. **Validates non-overlap with config dir** (ADR-073). If `workspace_path` contains or equals `~/.config/plexus/`, refuse with friendly stderr error → exit. Catches the dangerous "user set workspace_path to `~/.config/`" case before any disk activity.
2. **Auto-creates the directory if missing.** `tokio::fs::create_dir_all(workspace_path)` (mkdir -p semantics). Log `"Created workspace dir at <path>"` to stderr exactly once per process lifetime. mkdir failure (permissions, parent on a dead network mount) → friendly stderr error → exit.
3. **Accepts the directory as-is if it exists**, whether empty or non-empty. No marker file, no init metadata, no validation. Plexus does **not** "own" the workspace — the user can legitimately point it at an existing folder like `~/projects/myrepo/` and the agent operates on existing files in place. Pairs with the `unrestricted` use case where the workspace might be `~/` itself.

The "Plexus doesn't own the workspace" property means uninstall is just removing the binary + the config dir; user's files in the workspace are theirs and untouched.

#### Graceful shutdown — cancel immediately on SIGTERM/SIGINT

The client never tries to drain in-flight work on shutdown. On SIGTERM, SIGINT, or platform-equivalent (Windows console close):

1. Stop the worker queue from accepting new items (cancellation token flipped).
2. For each in-flight `tool_call` ID: send `tool_result(is_error=true, code='client_shutting_down', content='Client process is shutting down.')` over WS before closing.
3. For each in-flight transfer slot: send `transfer_end(id, ok=false, error='client_shutting_down')`.
4. Forceful kill on all MCP subprocesses and the in-flight `exec` subprocess (per ADR-105 teardown — `Child::start_kill()` cross-platform).
5. Close WS with code 1001 ("going away").
6. Exit zero.

Rationale for not draining: service managers (systemd default `TimeoutStopSec=90s`, launchd default 20s, Windows SCM variable) escalate SIGTERM → SIGKILL fast. A "drain for up to 10 minutes" model would just mean "drain for ~25s then OS force-kills you mid-cleanup, losing all the things you DID want to send." Cancel-immediately is honest about what we control. The agent receives the `client_shutting_down` errors → ADR-031 handles them → next reconnect resumes the session cleanly. Reconnect-after-restart already handles the "cargo build was running" case via the standard tool-failure → agent retries pattern.

#### Logging

Logs go to stderr. Service-manager environments (systemd journal, launchd unified log, Windows SCM) capture stderr automatically; interactive users redirect with shell piping or read it live.

**Backend:** `tracing` + `tracing-subscriber` (with `env-filter` + `time` features). Plain single-line text format in v1. JSON output is deferred — add a `--log-format=json` flag when there's a real ingestion-stack consumer (Loki, CloudWatch, ELK, etc.).

**Verbosity control:** `EnvFilter` with default `INFO`. Operators override via `RUST_LOG`:

```
RUST_LOG=debug ./plexus-client run                                 # everything at DEBUG
RUST_LOG=plexus_client=debug,plexus_common::mcp=trace ./plexus-client run   # targeted
```

Crate names use **underscores** in directives (`plexus_client`, not `plexus-client`) — this is `tracing-subscriber`'s convention. Document prominently or it becomes a "why doesn't my filter work" support burden. A convenience `--log-level=<level>` CLI flag is also accepted for users who don't want to learn `RUST_LOG` syntax; flag value seeds the filter and `RUST_LOG` overrides if both are set.

**Subscriber config:**

```rust
tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_ansi(false)                              // never emit color codes — stderr is usually redirected; Windows mangles them in files
    .with_timer(UtcTime::rfc_3339())               // UTC RFC3339 timestamps; same shape across all hosts; no local-tz drift
    .with_target(false)                            // hide module path on INFO+ for cleaner one-liners
    .with_file(false).with_line_number(false)      // file:line only at DEBUG/TRACE if the operator opts in
    .init();
```

**INFO inventory — state transitions and failures only.** Per-call logs go to DEBUG to avoid drowning the lifecycle signal at hundreds of calls/minute.

| Level | Logged |
|---|---|
| INFO | startup config summary (version, server URL host, workspace path); connection state changes (connect/disconnect/reconnect-attempt); MCP spawn/die/rejected with reason; sandbox-fallback-once; graceful shutdown observed |
| WARN | tool errors that surface to the agent; sandbox unavailability; heartbeat degradation; MCP crashes (Alive→Dead) |
| ERROR | startup failures (mkdir, env validation); WS handshake refusals; non-recoverable subprocess failures |
| DEBUG | every tool dispatch + completion; config_update reconciliation diff; register_mcp send |
| TRACE | frame-by-frame WS traffic; file-transfer chunk-by-chunk progress; MCP rmcp protocol traffic |

**No periodic "I'm alive" heartbeats at INFO.** Use metrics/external monitoring if needed.

**Structured fields, not format-string interpolation.** Use `tracing`'s typed-field syntax so the same call sites work cleanly when JSON output lands later:

```rust
// ❌ format-string interpolation:
info!("Tool {} dispatched (id={}, device={})", name, id, device);

// ✅ structured fields:
info!(tool = %name, id = %id, device = %device, "Tool dispatched");
```

Stable field names: `tool`, `mcp_id`, `attempt`, `pid`, `exit_code`, `server_url_host`, `device`, `error`. Avoid free-form keys.

**Secret redaction via the `secrecy` crate.** Every secret-bearing field on every struct uses `secrecy::SecretString` (with `zeroize` on drop). Custom `Debug`/`Display` impls exist on `SecretString` and never reveal the inner value — accidental `error!("config: {:?}", config)` is safe by construction. Affected fields:

- `device_token` (the `PLEXUS_DEVICE_TOKEN` env var)
- JWT bearer values
- `mcp_servers.<name>.env` values (MCP API keys live here per ADR-050)
- LLM `api_key` from `system_config` (server-side, ADR-101)

Test gate: assert no `plexus_dev_*` or JWT-shaped string ever appears in captured log output across a representative test suite. Keeps the "never log secrets" rule from regressing as new code lands.

#### Version mismatch — exit immediately, don't retry

Most reconnect failures are transient (server restart, network blip) and the client retries forever per the "Initial connect retry" rule above. **Protocol version mismatch is the one exception** — retrying with the same broken binary will never succeed, and looping pretends it might.

When the WS handshake closes with code `4409` (`version_unsupported`, see PROTOCOL.md §1.2), the close payload carries:

```jsonc
{
  "code": "version_unsupported",
  "server_version": "0.4.0",
  "protocol_version": "2",
  "client_minimum": "0.3.0",
  "upgrade_url": "https://github.com/<owner>/plexus/releases/tag/v0.4.0"
}
```

Client behavior:

1. **ERROR-level log** to stderr with the literal upgrade URL: *"Server requires plexus-client v0.3.0+ (server is v0.4.0, protocol v2). This client is v0.2.1, protocol v1. Download a newer client at https://github.com/.../releases/tag/v0.4.0 ."*
2. **Exit with code `78`** (`EX_CONFIG` from sysexits.h convention — "configuration error, don't bother restarting"). systemd users who want to suppress restart spam can add `RestartPreventExitStatus=78` to their unit file. We don't ship the unit file in v1 (per ADR-102) but document the suggestion in the README.
3. **Do NOT enter the reconnect loop.** This is the only WS close code that breaks the retry-forever rule. WS code 4401 (token revoked) is the same pattern — exit, don't retry — but documented separately in ADR-104's logout/auth area.

This pairs with ADR-102's M3 frontend integration: Settings → Devices in the web UI shows a download link pinned to the deployed server's version, so the user's "fix it" path is one click after they see the stderr message.

**Consequences:**
- Single startup contract: two env vars + one subcommand. Documents in 30 seconds.
- `logout` is a real action with a real server-side effect, not a placeholder.
- Sandbox fallback prioritizes "agent keeps working" over "fail fast" — admin sees the warning in logs and can fix later.
- Backoff-forever pairs cleanly with systemd / launchd / Windows service supervision; no separate "should I exit?" decision tree.
- Empty config dir keeps install/uninstall trivially `~/.config/plexus/` = the entire footprint.
- Workspace bootstrap supports both "fresh dir for Plexus" and "point at my existing repo" workflows without a config flag.
- Version mismatch fails fast and points users at the fix; doesn't generate restart-loop spam.

### ADR-107 · Versioning policy — pre-1.0 collapsed-tier; protocol version is independent

**Status:** accepted
**Context:** Plexus releases binaries for plexus-server and plexus-client per ADR-102. Two versioning concerns interact: the **binary release tag** (what shows in `plexus-client version` and on GitHub Releases), and the **protocol version** (what's sent in the WS `hello` frame and checked at handshake). Both need a clear policy so users, ops, and downstream tooling know what bumps mean.
**Decision:**

#### Phase 1 — pre-1.0 (M0 onward, current)

Binary release tags follow `0.m.x` with two-tier semantics (industry-common pre-1.0 / Cargo-ecosystem pattern):

- `0.m.x → 0.m.x+1` — backwards-compatible release. Bug fix or new feature, lumped together (the API is unstable anyway, distinguishing isn't worth the policy overhead).
- `0.m.x → 0.m+1.0` — potentially breaking change. Could be wire-protocol breaking, could be config schema breaking, could be a removed CLI flag.

This is **not** strict SemVer (which has three tiers: MAJOR/MINOR/PATCH). Strict SemVer would require us to distinguish "feature" from "fix" at every release; pre-1.0 projects rarely benefit from that distinction.

#### Phase 2 — post-1.0 (when API stabilizes)

When Plexus reaches `1.0.0`, switch to **full SemVer**:

- `n.m.x → n.m.x+1` — bug fix, backwards-compatible.
- `n.m.x → n.m+1.0` — feature, backwards-compatible.
- `n.m.x → n+1.0.0` — breaking change.

The `1.0.0` cutover is itself the signal that the API has stabilized; before then, "we might break things between minor versions" is the contract.

#### Protocol version is independent

The wire-protocol version (`hello.version` in PROTOCOL.md §1.2) is a **separate string**, not derived from the binary version. It bumps **only** when the WS frame format changes in a wire-incompatible way:

- Adding a new optional JSON field (e.g. `spawn_failures` on `register_mcp` per ADR-105) → no protocol bump. Old clients ignore the new field; new clients tolerate its absence.
- Renaming a frame, changing a field's type, removing a required field, adding a required field → protocol bump.

Most binary releases will NOT bump the protocol version — internal refactors, new tools, bug fixes, log changes, etc. don't touch the wire. The `4409` close code (handshake mismatch) only fires when the binary client genuinely speaks an older protocol the server can't accept.

This means a stale-but-not-too-stale client (e.g. binary `v0.3.0` speaking protocol `v1`, against server `v0.4.5` speaking protocol `v1`) keeps working — they just miss out on the new features baked into the newer binary's local code.

#### What goes where

- **Binary version** (`0.m.x`): GitHub release tag, `Cargo.toml` `version`, `plexus-client version` output, frontend Settings → Devices download links pinned to it.
- **Protocol version** (`"1"`, `"2"`, …): hardcoded constant in `plexus-common`, sent in `hello`, checked server-side at handshake. Server may accept multiple protocol versions during a transition window if the breaking change has a graceful migration path.
- **`4409` close payload** carries both, plus `client_minimum` and `upgrade_url`, so the client can render an actionable error message (per ADR-104).

**Consequences:**
- Pre-1.0 phase has a simple two-tier release rhythm; admins know `0.m+1.0` means "read the changelog before upgrading."
- Protocol version stays stable across most binary releases — most stale-client situations are silent feature-skip, not hard breakage.
- The 1.0 cutover is the natural "we're stable now" milestone; happens organically when the API has settled and we don't expect more breaking changes.
- README documents both versions: "plexus-client v0.3.1 (protocol v1)" so users know which to compare against the server.

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
| Per-user SSRF whitelist on `web_fetch` | Server: hardcoded block (no override). Client: per-device whitelist exceptions (capability declaration, not sandbox) | ADR-052 |
| `/api/files` ephemeral cache | Workspace canonical | ADR-044 |
| `vision_stripped` on session state | Retry at provider layer only | ADR-026 |
| Session = long-lived actor task + mpsc inbox | Session = DB row + transient lock | ADR-011 |
| `cascade_migrations` loop in `db/mod.rs` | Canonical `schema.sql` via `include_str!` | ADR-057 |
| Shell schema in `plexus-server/server_tools/` | Client owns; handshake-advertised | ADR-039 |
| File tool schemas in `plexus-server/server_tools/` | `plexus-common/tool_schemas/` | ADR-038 |
| MCP client code duplicated in server + client | Shared in `plexus-common/mcp/` | ADR-047 |

