# M2 Closeout Design Spec — Dream, Heartbeat, and Deferred Cleanup

**Date:** 2026-04-17
**Status:** Items A–B (dream, heartbeat) **superseded** by `2026-04-17-workspace-and-autonomy-design.md` — that spec covers the per-user workspace foundation plus dream and heartbeat. Items C–F remain fully scoped in this document.
**Goal:** Close the remaining gap between current Plexus and nanobot's "fully functional agent server" so M2 ships as complete.

---

## 1. Overview

Plexus M2 is currently 95% of nanobot-parity. The remaining 5% falls into six buckets:

**Nanobot-parity gaps (missing subsystems) — see `2026-04-17-workspace-and-autonomy-design.md`:**
- **A. Dream** — idle-triggered memory consolidation. Reads recent conversation history, updates long-term memory files in the user workspace, auto-discovers and writes reusable skills.
- **B. Heartbeat** — periodic task wake-up. Reads `HEARTBEAT.md` from the user workspace, the LLM decides skip/run via a virtual tool, runs selected tasks through the agent loop, evaluator-gated notify on external channels.
- **(Foundation)** — per-user server workspace (`{WORKSPACE_ROOT}/{user_id}/`), 11-tool server toolset, disk-as-truth skills, frontend Workspace page. All covered by the workspace-and-autonomy spec.

**Deferred-backlog closeout (explicitly M2):**
- **C. Account deletion** — self-serve + admin endpoints; teardown service; CASCADE migration; `kick_user` WS frame; frontend Danger Zone. (Full spec+plan already at `plans/2026-04-16-account-deletion.md` — this document references it, does not duplicate.)
- **D. Admin user-management UI** — list/search endpoint + Admin-page tab. Builds on the `DELETE /api/admin/users/{id}` endpoint that lands with (C).
- **E. Graceful shutdown for Discord/Telegram bots and per-session agent loops** — extend the existing cancellation-token fan-out (ADR-34) so bots and session loops participate in drain.
- **F. Session-list unread indicator** — small frontend fix so non-viewed sessions that receive a `session_update` frame get a visible dot in the session list.

Items A and B are the only ones that require new subsystems. C–F are cleanup of already-tracked deferred issues.

---

## 2. Scope

**In scope**

1. Dream subsystem: cron-scheduled per-user memory consolidation (two-phase).
2. Heartbeat subsystem: periodic per-user task wake-up (two-phase).
3. Account deletion (execute existing plan — referenced, not redesigned).
4. Admin user-management list/search UI + supporting endpoint.
5. Graceful-shutdown extension to Discord/Telegram bots and per-session agent loops.
6. Frontend session-list unread badge.

**Out of scope (stays deferred with documented reason)**

| Item | Origin | Why keep deferred |
|---|---|---|
| Manual E2E smoke tests for inbound-media, cross-channel addressing | server ISSUE.md | Not code tasks — need live Discord/Telegram bots; user-owned |
| Mid-ReAct-turn image re-read edge case | server ISSUE.md | No known failure; watch-only |
| Whisper / server-side voice transcription | server ISSUE.md | Explicit design decision 2026-04-15 — voice flows via `file_transfer` + client-side ASR |
| Last-admin invariant enforcement (ADR-33) | server ISSUE.md | Acceptable for small deployments; re-bootstrap via DB |
| Offline-browser push notifications (PWA service worker) | gateway ISSUE.md | Separate project; larger than M2 |
| Vitest / testing-library harness | frontend ISSUE.md | Testing infra, not a product capability |

---

## 3. Goals & Non-Goals

**Goals**

1. After this bundle lands, every row in every `ISSUE.md` under `## Open` is closed and items under `## Deferred` are either closed or carry an explicit "stays deferred" justification.
2. Nanobot's "dream" and "heartbeat" behaviors work in Plexus with the same user-visible semantics (periodic memory updates; periodic task checks).
3. Users can delete their own accounts end-to-end; admins can delete any user and can see a list of users to pick from.
4. A `SIGTERM` to `plexus-server` drains cleanly — no mid-turn aborts, no dropped Discord messages, no leaked bot poll tasks.
5. Cross-channel addressing (ADR-31) feels complete on the frontend — a notification in a background session is visible without opening it.

**Non-Goals**

- Reimagining memory from scratch. Dream edits structured files the LLM already knows how to edit; no new query/knowledge-graph layer.
- Cross-user memory or shared skills — per-user isolation holds.
- A scheduling DSL beyond what cron already supports.
- Push-notification infrastructure for offline browsers.
- A full CRUD admin panel — list + search + delete only.

---

## Part A — Dream Subsystem

> Design details pending brainstorm of **open question A1** (file-based vs DB-backed long-term memory surface). This section sketches the shape and invariants; prompts and storage layout are finalized after brainstorm.

### A.1 Purpose

Periodically consolidate each user's recent conversation history into durable long-term memory, and auto-discover reusable task patterns as skills. Mirrors `nanobot.agent.memory.Dream`.

### A.2 Trigger

- Registered as a **system cron job** per user at server boot (and at user creation), using the existing cron infrastructure. Job name: `"dream"`. Default cadence: every 2h (configurable per-user).
- Reuses the full claim-dispatch-reschedule pipeline (ADR-27). Dream dispatch publishes an `InboundEvent` with `session_key = "dream:{user_id}"` and a new `InboundEvent.kind = Kind::Dream`. The agent loop routes kinds (`Kind::UserTurn | Kind::Cron | Kind::Dream | Kind::Heartbeat`) to the correct execution path.
- Manual trigger: a new server endpoint `POST /api/admin/users/{user_id}/dream` and a user-facing `POST /api/user/dream` (self-serve).

### A.3 Two-Phase Flow

```
Phase 1 (Analysis)
  ├─ Load unprocessed messages since dream_cursors(user_id).last_message_id
  ├─ LLM call with templates/agent/dream_phase1.md
  │    inputs: history slice, current long-term-memory content (see A1)
  │    output: structured [FILE]/[FILE-REMOVE]/[SKILL] directives (text)
  └─ If directives empty → advance cursor, done.

Phase 2 (Execution)
  ├─ Spawn a restricted agent-loop sub-run with:
  │    - system prompt: templates/agent/dream_phase2.md + directives
  │    - tool allowlist: read_file, write_file, edit_file (scoped to the user's memory dir)
  │    - max iterations: lower cap (e.g. 30)
  ├─ Run to completion or cap.
  ├─ Advance dream_cursors.last_message_id regardless of partial failure
  └─ Compact/mark-compressed the processed messages (reuse memory.rs flow)
```

### A.4 Data Model

**New table: `dream_cursors`**

```sql
CREATE TABLE dream_cursors (
    user_id TEXT PRIMARY KEY
        REFERENCES users(user_id) ON DELETE CASCADE,
    last_message_id TEXT,          -- FK not enforced; messages are compressed/may vanish
    last_ran_at TIMESTAMPTZ,
    last_status TEXT CHECK (last_status IN ('ok','empty','failed')),
    UNIQUE (user_id)
);
```

CASCADE on `users` is required — account deletion (Part C) assumes every new per-user table cascades.

**Long-term memory surface** — see open question **A1**. Two candidate layouts:

| Option | Storage | Dream P2 tool set |
|---|---|---|
| A1a **File-based** (nanobot parity) | `$PLEXUS_SKILLS_DIR/{user_id}/memory/{MEMORY,SOUL,USER}.md` | `read_file`, `write_file`, `edit_file` on that subtree |
| A1b **DB-backed** | `users.memory_md`, `users.soul_md`, `users.user_md` (TEXT columns, already partially present) | New `memory_edit(section, op, content)` tool on a synthetic in-memory filesystem |

A1a is cheaper to port (prompts work as-is) but adds disk state to back up. A1b keeps Plexus single-store but needs a new tool and diverges from nanobot prompts. Decision deferred to brainstorm.

### A.5 Prompts

Port:
- `nanobot/templates/agent/dream_phase1.md` → `plexus-server/templates/prompts/dream_phase1.md`
- `nanobot/templates/agent/dream_phase2.md` → `plexus-server/templates/prompts/dream_phase2.md`

Minor edits to replace nanobot-specific tool/path language with Plexus equivalents. Exact edits depend on A1.

### A.6 Context Builder Changes

`plexus-server/src/context.rs::build_context` currently takes no mode flag. Add:

```rust
pub enum PromptMode {
    UserTurn,     // existing behavior
    Dream,        // suppress channels/skills/devices sections; inject dream prompt
    Heartbeat,    // suppress skills/devices; inject heartbeat prompt
}

pub async fn build_context(..., mode: PromptMode) -> ... { ... }
```

Dream mode short-circuits most optional sections — no per-session channel identity, no per-user device list. Only the memory surface, current time, and the dream-specific instructions are needed.

### A.7 Skill Creation

Dream Phase 2 can write to `$PLEXUS_SKILLS_DIR/{user_id}/{skill_name}/SKILL.md` (existing per-user skill layout). The skill-registry cache already invalidates when files change on disk, so new skills are picked up on next session boot without a service restart.

### A.8 Invariants

- Dream never reads cross-user data.
- Dream cursor advances even if Phase 2 partially fails, so a bad batch doesn't block subsequent runs (mirrors nanobot).
- Dream runs do not appear in user-facing session history (session_key uses `dream:` prefix, excluded from session list).
- Dream respects the per-user rate limiter (counts against the same quota).

---

## Part B — Heartbeat Subsystem

> Design details pending brainstorm of **open question B1** (HEARTBEAT.md location/format). Shape and invariants below are fixed; exact layout is finalized after brainstorm.

### B.1 Purpose

Every N minutes per user, check a user-owned "things I want done in the background" list and execute ripe tasks. Mirrors `nanobot.heartbeat.service`.

### B.2 Trigger

- One async tick loop owned by `AppState`, not cron. (Nanobot uses cron; Plexus uses a dedicated tick because heartbeat's per-user schedule is simple — one interval, no expressions — and the claim-reschedule dance isn't needed for an in-process timer.)
- Per-user interval stored in `users.heartbeat_interval_seconds` (default 1800; 0 = disabled).
- The tick loop iterates users where `now - last_heartbeat_at >= interval`, dispatches an `InboundEvent { kind: Heartbeat, session_key: "heartbeat:{user_id}" }` to the bus, and updates `last_heartbeat_at`.

### B.3 Two-Phase Flow

```
Phase 1 (Decision)
  ├─ LLM call with:
  │    - system prompt: "you are a heartbeat agent. call the heartbeat tool."
  │    - user content: HEARTBEAT.md body + current time (user timezone)
  │    - tools: [heartbeat(action: "skip"|"run", tasks: string)]
  ├─ If action="skip" → done. Record noop.
  └─ If action="run" → extract `tasks` string, enter Phase 2.

Phase 2 (Execution)
  ├─ Run the normal agent loop with:
  │    - session_key: "heartbeat:{user_id}"
  │    - user message: `tasks` from Phase 1
  │    - system prompt mode: Heartbeat
  │    - tool allowlist: full agent toolset (same as a user turn)
  ├─ Run to completion.
  └─ Evaluator post-pass:
       - small LLM call: "did this produce output worth pinging the user?"
       - if yes → emit OutboundEvent on a non-gateway channel (Discord/Telegram)
       - never notify via gateway (would disrupt active browser session)
```

### B.4 Data Model

**User columns (migration):**

```sql
ALTER TABLE users
  ADD COLUMN heartbeat_interval_seconds INT NOT NULL DEFAULT 1800,
  ADD COLUMN last_heartbeat_at TIMESTAMPTZ;
```

**HEARTBEAT list location** — see open question **B1**. Two candidate layouts:

| Option | Storage |
|---|---|
| B1a **File-based** (nanobot parity) | `$PLEXUS_SKILLS_DIR/{user_id}/HEARTBEAT.md` — dream can edit it naturally |
| B1b **DB-backed** | `users.heartbeat_md TEXT` |

Tied to A1: file if dream goes file-based; DB if dream stays DB-backed.

### B.5 Prompts

Port the hardcoded Phase 1 system message from `nanobot/heartbeat/service.py:96`, verbatim where possible. Phase 2 reuses the normal Heartbeat-mode system prompt.

Evaluator prompt is new: a short "was this output user-worthy?" classifier that returns `{"notify": bool, "reason": string}`. Inline-defined, no separate template.

### B.6 Delivery Rules

- Heartbeat **never** publishes to the gateway channel (would interrupt the browser UX).
- Heartbeat publishes to the user's primary external channel (Discord or Telegram), preferring Discord if both configured.
- If no external channel is configured, the result is logged and discarded; the user can read past heartbeat sessions by session-key-prefix search (future admin feature, out of scope here).

### B.7 Invariants

- The tick loop is a single task; it is a no-op for users with `heartbeat_interval_seconds = 0`.
- Heartbeat never fires for a user who already has a heartbeat session mid-turn (check session-inbox length; skip if nonzero).
- Heartbeat respects user rate limits (same quota as user turns).
- `ON DELETE CASCADE` from `users` removes `last_heartbeat_at` implicitly (column-level).

---

## Part C — Account Deletion (execute existing plan)

No redesign. Execute `docs/superpowers/plans/2026-04-16-account-deletion.md` as-is (AD-1 through AD-9).

**This bundle adds one requirement to the plan:** the CASCADE migration (task AD-2) must include:

- `dream_cursors(user_id) REFERENCES users(user_id) ON DELETE CASCADE`
- (Any other per-user tables added by Parts A/B follow the same rule.)

If `dream_cursors` lands before AD-2 runs, the migration file grows to cover it. If after, the next migration adds CASCADE retroactively.

**Removes deferred issue** `SessionHandle.user_id` scaffolding — becomes load-bearing in the `evict_in_memory` step and `#[allow(dead_code)]` gets dropped.

---

## Part D — Admin User-Management UI

### D.1 Endpoint

```
GET /api/admin/users?search=<q>&limit=<n>&offset=<o>
   → 200 { users: [{user_id, email, display_name, is_admin,
                    created_at, last_active_at}], total: N }
   auth: Bearer admin JWT
```

- `search` matches on email prefix and display_name substring (ILIKE). Empty = list all.
- `limit` defaults 50, max 200. `offset` for simple pagination.
- `last_active_at` = `MAX(sessions.last_message_at)` joined in.

No POST / PATCH. Admin user creation stays via bootstrap + registration flow (existing). Deletion uses the existing `DELETE /api/admin/users/{user_id}` from Part C.

### D.2 Frontend

New Admin-page tab "Users" (beside existing "Skills" / "LLM Config" etc.):

- Search box (debounced, 300 ms).
- Table: email | display_name | admin? | created | last active | 🗑️.
- Delete icon opens a confirm modal: "Delete {email}? Type DELETE to confirm." Calls `DELETE /api/admin/users/{id}` and refreshes the list.
- Pagination: prev/next buttons at 50/page.

No in-place edit, no sorting UI (default sort: `last_active_at DESC NULLS LAST`). Keep surface minimal — this is admin plumbing, not a CRM.

### D.3 Invariants

- Admin deleting themselves works (per ADR-33) but the row is removed from the table optimistically, triggering a redirect to `/login` (their JWT is now invalid).
- List excludes soft-deleted rows — there is no soft delete, so this is a no-op note.

---

## Part E — Graceful Shutdown for Bots and Per-Session Loops

### E.1 Motivation

ADR-34 wired `AppState.shutdown: CancellationToken` into the 5 main background loops and HTTP. Two surfaces were left out (documented as deferred ISSUE):

- Discord and Telegram bot poll loops continue reading inbound messages during drain.
- Per-session agent loops keep running their current ReAct iteration with no cooperative checkpoint.

### E.2 Design

**Bots.** Each bot (`channels::discord::start_bot`, `channels::telegram::start_bot`) accepts an additional `CancellationToken` at construction. Their main event loops use `tokio::select!` between the bot event and `shutdown.cancelled()`. On cancel: stop reading new events, drain in-flight inbound publishes (<= 1s), then exit.

**Per-session agent loops.** `agent_loop::run_session` already has a `loop { match inbox_rx.recv().await { ... } }`. Extend to:

```rust
loop {
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => {
            // Current turn: allow it to finish (inner iterate() is not interruptible safely —
            // tool calls in flight on client devices must complete or we corrupt state).
            // Block new inbox events from processing.
            break;
        }
        event = inbox_rx.recv() => { ... handle ... }
    }
}
```

- The `biased` selector prevents inbox events from starving cancellation.
- Current-turn interruption is **not** safe — remote tool invocations are already in flight to clients and need their responses collected. Instead, we accept that `SIGTERM` waits up to the existing HTTP `graceful_shutdown` grace period (currently 30s) for in-flight turns to drain.
- If a turn is still running at the grace-period cutoff, the session is abandoned and the DB-based crash-recovery path (ADR-10) handles the partial state on next boot.

**Drain order (updated ADR-34 fan-out):**

```
SIGTERM
  → shutdown.cancel()
  → bots: stop reading (immediate)
  → sessions: stop accepting new events (drains within grace period)
  → HTTP server: stop accepting; existing requests finish
  → existing 5 loops: exit on select!
  → final 5-second hard timeout, then abort
```

### E.3 Invariants

- No new message is dispatched to a session after `shutdown.cancel()`.
- No bot produces a new `InboundEvent` after `shutdown.cancel()`.
- A turn in progress either completes and saves, or leaves durable partial state that crash-recovery resumes.

---

## Part F — Frontend Session-List Unread Badge

### F.1 Current Behavior

Gateway `session_update` frame (ADR-31) reaches the frontend and the chat store calls `refreshSession(sessionId)` for any session, including non-viewed ones. The store is updated, but there is no visual indicator.

### F.2 Change

**Chat store (Zustand):**

```ts
type Session = { id: string; title: string; lastMessageAt: number; hasUnread: boolean; ... };

onSessionUpdate(sessionId): void {
  if (sessionId === currentSessionId) {
    refreshSession(sessionId);  // existing path
  } else {
    refreshSession(sessionId);
    setSessionUnread(sessionId, true);
  }
}

openSession(sessionId): void {
  currentSessionId = sessionId;
  setSessionUnread(sessionId, false);
}
```

**Session list component:** render a 6px dot to the right of the title when `hasUnread === true`. Tailwind: `bg-blue-500 rounded-full`.

**Persistence:** `hasUnread` is local store state. It resets on refresh (fine — on page load, the user's viewport is the source of truth for "what I've seen").

### F.3 Invariants

- The dot clears as soon as the session is opened.
- Opening the current session (clicking the already-active session) is a no-op visually; the flag is already false.
- `session_update` for the current session never sets `hasUnread`.

---

## 4. Cross-Cutting Concerns

### 4.1 New `InboundEvent.kind` enum

The bus currently distinguishes events by `cron_job_id: Option<...>`. For dream + heartbeat to share the per-session agent-loop path while using different prompts/tool sets, introduce:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventKind { UserTurn, Cron, Dream, Heartbeat }

pub struct InboundEvent {
    pub session_id: String,
    pub user_id: String,
    pub kind: EventKind,
    // existing fields …
}
```

`cron_job_id` stays (needed for cron rescheduling). `kind` is the dispatch discriminant for `context.rs` and tool-allowlist gating.

### 4.2 Prompt-mode dispatch

`context::build_context(..., mode: PromptMode)` replaces the current implicit single mode. Call-sites:

- agent_loop `UserTurn` → `PromptMode::UserTurn`
- agent_loop `Cron` → `PromptMode::UserTurn` (cron uses normal context)
- agent_loop `Dream` → `PromptMode::Dream`
- agent_loop `Heartbeat` → `PromptMode::Heartbeat`

### 4.3 Tool allowlisting

Introduce a simple allowlist enforced in the tool dispatcher:

```rust
pub enum ToolAllowlist { All, Only(HashSet<&'static str>) }

fn dispatch_tool(name: &str, allow: &ToolAllowlist) -> Result<...> {
    match allow {
        ToolAllowlist::All => proceed,
        ToolAllowlist::Only(set) if set.contains(name) => proceed,
        _ => Err(ToolError::NotAllowedInMode),
    }
}
```

Dream P2 uses `Only({read_file, write_file, edit_file})`; everything else uses `All`.

### 4.4 Migrations

One migration file adds:
- `dream_cursors` table (Part A).
- `users.heartbeat_interval_seconds`, `users.last_heartbeat_at` (Part B).
- Optional `users.heartbeat_md` and `users.memory_md`/`users.soul_md`/`users.user_md` if we land on DB-backed layouts (A1/B1 decision).

All new per-user rows/columns CASCADE on user deletion — this is a hard rule going forward.

---

## 5. Open Design Questions (to resolve in brainstorm)

- **A1 — Long-term memory surface: file-based vs DB-backed.** File-based is closer to nanobot and reuses their prompts verbatim. DB-backed keeps the "PostgreSQL is the sole persistent store" invariant (ADR-4) but diverges from nanobot prompts and needs a new tool. _Recommendation in investigation: file-based (lowest port cost), but requires accepting `$PLEXUS_SKILLS_DIR` as a non-trivial data directory._
- **B1 — HEARTBEAT list surface.** Tied to A1. Same tradeoff.
- **A2 — Dream cadence default.** Nanobot defaults 2h. Appropriate for chat-heavy users; potentially too aggressive for a user with one 5-minute session a week. Options: fixed 2h, adaptive (trigger after N unprocessed messages), or user-tunable (`users.dream_interval_seconds`).
- **B2 — Heartbeat per-user vs single loop.** One tick task iterating users every 60s vs per-user `tokio::time::interval`. One-loop is simpler; per-user is more precise. Likely one-loop wins — heartbeat is not time-critical.
- **A3 — Dream's Phase 2 session key retention.** Should dream sessions persist in the DB after the run, or be purged? Nanobot keeps a rolling window. Plexus convention: keep them but tag with kind for admin filtering.

---

## 6. Proposed New ADRs (to draft after brainstorm)

- **ADR-35**: Dream subsystem as a cron-registered per-user job (not a separate scheduler).
- **ADR-36**: Heartbeat as a dedicated in-process tick loop (not cron).
- **ADR-37**: Long-term memory surface — _file-based OR DB-backed_ (pending A1).
- **ADR-38**: EventKind discriminant on `InboundEvent` and prompt-mode dispatch in `build_context`.
- **ADR-39**: Admin user-management list scope — list + search + delete; no CRUD.
- **ADR-40**: Graceful shutdown extension — bots and per-session loops participate in fan-out; in-flight turns granted up to the HTTP grace window.

ADR bodies are drafted once the design brainstorm resolves A1/B1.

---

## 7. Execution Order

Plans are split per part for independent execution. Recommended order:

1. **C** (account deletion — existing plan, zero open questions) — lands CASCADE infra and `SessionHandle.user_id` usage first. Other parts depend on CASCADE being in place.
2. **F** (unread badge — frontend-only, ~1 day) — quick win, unblocks ADR-31 UX completeness.
3. **D** (admin user-management UI — depends on C's DELETE endpoint).
4. **E** (graceful shutdown extension — touches agent loop + bots; isolated change, no new subsystems).
5. **A** (dream — after A1 brainstorm + ADR-35/37 written).
6. **B** (heartbeat — after B1 brainstorm + ADR-36 written; shares prompt-mode infra with dream so should follow A).

Parts A and B share `EventKind`, `PromptMode`, and tool-allowlist plumbing. Whichever lands first introduces all three; the other reuses.

---

## 8. Success Criteria

- `ISSUE.md` across all four crates has **no `## Open` items** after this bundle lands. Every `## Deferred` item is either closed or annotated with a "stays deferred — {reason}" line that matches section 2's out-of-scope table.
- A test user can: (1) send a message → dream runs on cadence → next conversation sees updated MEMORY surface; (2) add a task to HEARTBEAT → receive a Discord ping when it's handled.
- `SIGTERM` during a cron-driven ReAct turn leaves the DB in a state that the next boot recovers from; Discord inbound drains cleanly.
- Admin can locate any user via search in ≤ 2 clicks and delete them.
- The session list shows a dot on any non-current session that has received a `session_update` frame since last view.
- New per-user tables all CASCADE to `users`. No orphan rows possible.

---

## 9. Revision Log

- **2026-04-17** — Initial draft. Parts A/B sketched, C–F fully scoped. Open questions A1/B1/A2/B2/A3 flagged for dream/heartbeat brainstorm.
