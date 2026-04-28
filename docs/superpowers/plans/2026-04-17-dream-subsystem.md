# Dream Subsystem Implementation Plan (Plan D of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Spec reference:** The full design lives at `/home/yucheng/Documents/GitHub/Plexus/docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md` §8 + §11. Read it if this plan's context seems incomplete.
>
> **This is Plan D of 5.** Prior plans: **A** (workspace foundation, `e6f1da4..2fe90a0`) and **C** (shared evaluator + cron integration, `2464692..6643b0c`). Remaining after this: **E** heartbeat subsystem (reuses the `EventKind`/`PromptMode`/`ToolAllowlist` scaffolding introduced here), **B** frontend Workspace page.

**Goal:** Implement nanobot-parity dream — periodic memory consolidation and skill discovery — as a protected system cron job per user, with an idle check at fire time, a two-phase LLM pipeline (analysis → execution), and a restricted file-only tool allowlist for Phase 2.

**Architecture:** Plan C's `ensure_system_cron_job` registers a `{name:"dream", kind:"system", deliver:false}` cron job for each user at registration. When the cron fires, a new `dream::handle_dream_fire` orchestrator runs inline in the cron poller: cheap idle check (skip if no activity since `last_dream_at`), then Phase 1 (standalone LLM call with `dream_phase1.md` + history + `MEMORY.md` + `SOUL.md` + skills index) emits structured directives. Non-empty directives are published as an `InboundEvent { kind: EventKind::Dream, session_id: "dream:{user_id}", content: directives }`. The agent loop routes `EventKind::Dream` to `PromptMode::Dream` (system prompt = `dream_phase2.md`) with `ToolAllowlist::Only(file_tools)` so Phase 2 can read/edit `MEMORY.md`/`SOUL.md` and `write_file`/`delete_file` skills but cannot `message`, `cron`, or `web_fetch`. Cron's post-run `publish_final` (C-2) silently skips because `deliver=false`.

**Tech Stack:** Rust 1.85 (edition 2024), tokio, sqlx (PostgreSQL), serde_json, tracing. Reuses `evaluator.rs` LLM-call plumbing for Phase 1's direct call; reuses `agent_loop`'s `run_session` for Phase 2.

**Parent branch:** current `M3-gateway-frontend`, based on commit `6643b0c` (C-5 TOCTOU fix).

---

## 1. Overview

Dream has three cooperating pieces:

1. **Registration.** At user registration, `workspace::registration::initialize_user_workspace` calls Plan C's `ensure_system_cron_job` with `{name: "dream", cron_expr: "0 */2 * * *", timezone: user's tz, deliver: false, kind: system}`. Every user gets exactly one protected dream job. C-3 + C-4 prevent users from deleting it.

2. **Fire handler.** When the cron poller claims a `kind='system' AND name='dream'` job, it bypasses the usual `publish_inbound(InboundEvent{kind: Cron, ...})` flow and instead calls `dream::handle_dream_fire(state, job)`. The handler:
   - Reads `system_config.dream_enabled`. Skip if false.
   - Computes `last_activity_at` for the user from the `messages` table (excluding `dream:*` and `heartbeat:*` sessions).
   - If `last_activity_at <= last_dream_at` — no new activity — skip.
   - Update `users.last_dream_at = NOW()` now (prevents refire during Phase 1/2 duration).
   - Run Phase 1 inline: direct LLM call (no tools) with `dream_phase1.md` + history slice (messages after previous `last_dream_at`, bounded to 200) + current `MEMORY.md` + current `SOUL.md` + skills index.
   - If Phase 1 returns `[NO-OP]` or empty directives, we're done.
   - Otherwise publish an `InboundEvent { kind: Dream, session_id: "dream:{user_id}", content: directives, cron_job_id: Some(job.job_id), ... }`.

3. **Phase 2 agent-loop run.** The agent loop picks up the `EventKind::Dream` event. `build_context` routes on `PromptMode::Dream` and assembles a system prompt using `dream_phase2.md` + `MEMORY.md` + `SOUL.md` + skills index (but NOT channels/devices). The tool dispatcher uses `ToolAllowlist::Only(&["read_file","write_file","edit_file","delete_file","list_dir","glob","grep"])` — any other tool call returns an error. Max iterations: 30. When the agent loop terminates with a final `LlmResponse::Text`, `publish_final(cron_job_id=Some(dream_job_id), job_deliver=Some(false))` skips the evaluator entirely (C-2's cron deliver=false branch), so the final message is logged but never published. `reschedule_after_completion` fires as normal.

This plan introduces three cross-cutting scaffolds that Plan E will also consume:

- **`bus::EventKind` enum** with variants `UserTurn | Cron | Dream | Heartbeat` (Heartbeat added now so Plan E doesn't need to bump this file).
- **`context::PromptMode` enum** with variants `UserTurn | Dream | Heartbeat` (ditto).
- **`server_tools::ToolAllowlist` enum** with variants `All | Only(&'static [&'static str])` used by the tool dispatcher.

## 2. Goals & Non-Goals

**Goals**

- Dream runs once per activity window per user, with near-zero cost on idle users (one DB query per 2h per user).
- Dream cannot publish to channels — file edits only.
- Phase 2 cannot use `message`, `cron`, `file_transfer`, `web_fetch` — enforced by `ToolAllowlist`.
- `MEMORY.md` section structure (`## User Facts`, etc.) is preserved across dreams; the analysis prompt teaches this convention.
- Skills created by dream have the correct frontmatter format (Plan A's `create_skill` skill format).
- Administrators can disable dream globally via `system_config.dream_enabled = false` (already seeded by A-20).
- Administrators can override the Phase 1 / Phase 2 prompts via `system_config.dream_phase1_prompt` / `dream_phase2_prompt`; fallback is `include_str!`.

**Non-Goals**

- Per-user dream cadence. All users share the same cron expression (`0 */2 * * *`); admin can tweak by editing the system_config, but there's no per-user knob.
- `USER.md` (nanobot's third workspace file). Plexus folds user facts into `MEMORY.md` sections.
- Mid-dream interruption by user messages. If a user messages during a dream run, their session inbox queues normally; the dream session is separate.
- Cross-user dream influence. Each user's dream reads only their own sessions/memory/soul.
- Tests that stand up a real LLM. Phase 1 / Phase 2 integration tests are deferred; unit tests cover the idle-check, directive parsing, and allowlist behavior.
- Evaluator gating for dream. Dream's cron job has `deliver=false`, so `publish_final` (C-2) short-circuits before the evaluator path. Dream is always silent to channels.
- Exposing `EventKind::Heartbeat` consumers (beyond the enum variant existing). Plan E wires up the consumer.

## 3. Design

### 3.1 `EventKind` enum

```rust
// plexus-server/src/bus.rs

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EventKind {
    UserTurn,
    Cron,
    Dream,
    Heartbeat,
}

pub struct InboundEvent {
    pub session_id: String,
    pub user_id: String,
    pub kind: EventKind,                   // NEW
    pub content: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub media: Vec<String>,
    pub cron_job_id: Option<String>,       // still needed for reschedule_after_completion
    pub identity: Option<ChannelIdentity>,
}
```

All 5 existing constructors set `kind` explicitly:

| Site | Kind |
|---|---|
| `channels/gateway.rs:145` | `UserTurn` |
| `channels/discord.rs:340` | `UserTurn` |
| `channels/telegram.rs:285` | `UserTurn` |
| `cron.rs:57` (regular cron jobs) | `Cron` |
| `dream.rs::handle_dream_fire` (new) | `Dream` |
| `agent_loop.rs:614, 637` (tests) | explicit |

**`cron_job_id` is retained** — the field is still needed for `reschedule_after_completion` to know which job to reschedule after a cron/dream turn ends. It's no longer a dispatch discriminant: `kind` serves that purpose.

**Rate-limit exemption** at `bus::publish_inbound:38` currently uses `event.cron_job_id.is_none()` to decide whether to rate-limit. Change to `event.kind == EventKind::UserTurn` so dream and heartbeat also bypass the user's per-minute rate limit (they aren't user-initiated).

**ToolContext `is_cron`** at `server_tools/mod.rs:215` is derived from `cron_job_id.is_some()`. Change to `event.kind == EventKind::Cron` — a dream or heartbeat turn is NOT a cron turn from the tool's perspective (e.g., the `cron` tool's nested-scheduling guard should only apply to regular cron contexts; dream Phase 2 can't use `cron` anyway because of `ToolAllowlist`, so this is mostly cosmetic — but cleaner semantics).

### 3.2 `PromptMode` enum

```rust
// plexus-server/src/context.rs

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PromptMode {
    UserTurn,
    Dream,
    Heartbeat,
}
```

`build_context` signature extended with `mode: PromptMode` as the final parameter. Default-compatible callers pass `UserTurn`.

**`PromptMode::Dream` branch** (inside `build_context`):
- Soul: read `{workspace}/soul.md` + append system-default soul fallback (same as UserTurn).
- Memory: read `{workspace}/MEMORY.md` (same).
- Skills index: always-on skills inline + on-demand skills indexed (same).
- **Omit** channel identity section, device list, current time. Dream prompts don't need them.
- System-prompt prefix is `dream_phase2_prompt` (admin override) or `include_str!("../templates/prompts/dream_phase2.md")` (default).

**`PromptMode::Heartbeat`** is a placeholder for now — returns the same layout as `UserTurn`. Plan E finalizes.

### 3.3 `ToolAllowlist`

```rust
// plexus-server/src/server_tools/mod.rs

#[derive(Debug, Clone)]
pub enum ToolAllowlist {
    /// Every registered server tool is dispatchable.
    All,
    /// Only tools whose names appear in the slice may dispatch.
    /// Any other tool call returns a structured error before touching the tool function.
    Only(&'static [&'static str]),
}

impl ToolAllowlist {
    pub fn allows(&self, tool_name: &str) -> bool {
        match self {
            ToolAllowlist::All => true,
            ToolAllowlist::Only(names) => names.contains(&tool_name),
        }
    }
}

pub const DREAM_PHASE2_ALLOWLIST: &[&str] = &[
    "read_file", "write_file", "edit_file", "delete_file",
    "list_dir", "glob", "grep",
];
```

**Dispatch site** in `agent_loop.rs` (where `server_tools::execute(state, ctx, tool_name, args)` is called) gains an `allowlist` parameter. Before forwarding to `execute`, check:

```rust
if !allowlist.allows(tool_name) {
    let err = format!(
        "Tool '{tool_name}' is not available in this context (dream phase 2). \
         Available tools: read_file, write_file, edit_file, delete_file, list_dir, glob, grep."
    );
    // Save the error as the tool result and continue the loop so the LLM can recover.
    save_tool_result(err);
    continue;
}
```

Client-side tool routing (`tools_registry::route_to_device`) is ALSO filtered — dream cannot invoke client tools (shell, read_file on a device, etc.). The allowlist check applies BEFORE the server-vs-client routing split.

**The allowlist is passed into `run_session`** as part of session setup; it's constant for the session's lifetime.

### 3.4 Dream handler (`dream.rs`)

```rust
pub async fn handle_dream_fire(
    state: &Arc<AppState>,
    job: &crate::db::cron::CronJob,
) -> Result<(), String> {
    // 1. Global kill switch
    if !is_dream_enabled(&state.db).await {
        info!(user_id = job.user_id, "dream: globally disabled, skipping");
        return Ok(());
    }

    // 2. Idle check
    let last_activity = crate::db::messages::last_activity_for_user(&state.db, &job.user_id).await
        .map_err(|e| format!("last_activity_for_user: {e}"))?;
    let last_dream = crate::db::users::get_last_dream_at(&state.db, &job.user_id).await
        .map_err(|e| format!("get_last_dream_at: {e}"))?;

    let should_dream = match (last_activity, last_dream) {
        (None, _) => false,               // user has no activity at all
        (Some(_), None) => true,          // first dream ever
        (Some(a), Some(d)) => a > d,      // new activity since last dream
    };
    if !should_dream {
        return Ok(());
    }

    // 3. Claim this fire window by advancing last_dream_at BEFORE running phases
    let now = chrono::Utc::now();
    crate::db::users::update_last_dream_at(&state.db, &job.user_id, now).await
        .map_err(|e| format!("update_last_dream_at: {e}"))?;

    // 4. Phase 1: standalone LLM call
    let directives = run_phase1(state, &job.user_id, last_dream).await;
    if directives.trim().is_empty() || directives.contains("[NO-OP]") {
        info!(user_id = job.user_id, "dream: no-op directives, skipping phase 2");
        return Ok(());
    }

    // 5. Phase 2: publish InboundEvent, agent_loop handles the rest
    let event = InboundEvent {
        session_id: format!("dream:{}", job.user_id),
        user_id: job.user_id.clone(),
        kind: EventKind::Dream,
        content: directives,
        channel: job.channel.clone(),
        chat_id: Some(job.chat_id.clone()),
        media: vec![],
        cron_job_id: Some(job.job_id.clone()),
        identity: None,
    };
    crate::bus::publish_inbound(state, event).await
        .map_err(|e| format!("dream: publish_inbound: {e}"))?;

    Ok(())
}
```

`run_phase1` is a direct LLM call that mirrors the evaluator's pattern:
- Load `MEMORY.md`, `SOUL.md` content (empty string fallback).
- Glob skills index (name + description only, not full content).
- Fetch `messages` rows since `last_dream` (or since epoch if first dream), excluding `dream:*` / `heartbeat:*` sessions. Cap at 200 messages.
- Build a single-turn chat with `dream_phase1.md` as system prompt and the bundled inputs as the user message.
- Call `providers::openai::call_llm(client, config, messages, None, None)` — no tools.
- Return the LLM's text output as `directives`.

On any error (LLM unreachable, config missing, etc.), log a `warn!` and return empty string (skip Phase 2).

### 3.5 Cron poller routing

In `cron.rs::poll_and_execute`, after `claim_due_jobs` returns the list:

```rust
for job in claimed {
    info!("Cron firing: {} [{}] kind={}", job.name, job.job_id, job.kind);

    // Route by kind + name. Extensible: future system jobs can add arms here.
    if job.kind == crate::db::cron::SYSTEM_KIND && job.name == "dream" {
        let state = Arc::clone(state);
        let job_clone = job.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::dream::handle_dream_fire(&state, &job_clone).await {
                warn!(job_id = %job_clone.job_id, "dream handler error: {e}");
            }
            // Dream handler is responsible for publishing if it chose to proceed.
            // Rescheduling happens via agent_loop's reschedule_after_completion
            // when the Dream InboundEvent completes its turn. If dream skipped
            // (idle check false OR NO-OP directives), we still need to reschedule
            // — but no turn ran, so trigger rescheduling inline here.
            //
            // Since there's no direct way to know from handle_dream_fire whether
            // it published or not (both paths return Ok), the CLEANEST is to
            // always reschedule the cron AFTER a skip. For the published case,
            // reschedule_after_completion runs twice (once from us, once from
            // the agent loop) — but both write the same NOW()-derived next_run_at,
            // so it's idempotent modulo a trivial race.
            //
            // We SKIP the inline reschedule here and instead have handle_dream_fire
            // call it explicitly when it short-circuits.
        });
        continue;
    }

    // Regular cron job: publish as today.
    let event = InboundEvent {
        session_id: format!("cron:{}", job.job_id),
        user_id: job.user_id.clone(),
        kind: EventKind::Cron,
        content: job.message.clone(),
        channel: job.channel.clone(),
        chat_id: Some(job.chat_id.clone()),
        media: vec![],
        cron_job_id: Some(job.job_id.clone()),
        identity: None,
    };
    // ... existing publish_inbound + error handling
}
```

**Handler-controlled rescheduling:** `handle_dream_fire` is responsible for calling `reschedule_after_completion(state, job_id, true)` on any path that does NOT publish (idle skip, NO-OP directives, LLM error). On paths that DO publish, the agent_loop's existing `reschedule_after_completion` fires naturally when the dream turn completes. `reschedule_after_completion` is idempotent enough that duplicate calls don't cause duplicates (it writes `next_run_at`, not append-only).

### 3.6 Context builder PromptMode branch

Inside `build_context` (`context.rs`), add a conditional on `mode`:

```rust
// Choose system-prompt prefix.
let system_prefix = match mode {
    PromptMode::UserTurn => {
        // Existing behavior: soul + identity + channels + devices + time.
        // ... keep existing code here ...
    }
    PromptMode::Dream => {
        // Dream phase 2: prompt template + skills context only.
        let prompt = state.dream_phase2_prompt.read().await.clone();
        // Suffix MEMORY.md + SOUL.md + skills index inline.
        format!(
            "{prompt}\n\n## Current MEMORY.md\n\n{memory}\n\n## Current SOUL.md\n\n{soul}\n\n{skills_section}"
        )
    }
    PromptMode::Heartbeat => {
        // Plan E: stub for now, returns UserTurn's shape.
        // Same as UserTurn.
    }
};
```

`state.dream_phase2_prompt: Arc<RwLock<String>>` is initialized at boot by reading `system_config.dream_phase2_prompt` (admin override), falling back to `include_str!("../templates/prompts/dream_phase2.md")`. Similarly `dream_phase1_prompt`. These are NOT hot-reloaded at runtime (admin-tune window).

### 3.7 `users.last_dream_at` column

```sql
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_dream_at TIMESTAMPTZ;
```

DB helpers:

```rust
// plexus-server/src/db/users.rs
pub async fn get_last_dream_at(pool: &PgPool, user_id: &str)
    -> sqlx::Result<Option<chrono::DateTime<chrono::Utc>>>;

pub async fn update_last_dream_at(pool: &PgPool, user_id: &str,
    at: chrono::DateTime<chrono::Utc>) -> sqlx::Result<()>;
```

And a message-table helper:

```rust
// plexus-server/src/db/messages.rs
/// Most recent message timestamp across all non-autonomous sessions of a user.
/// Excludes dream:* and heartbeat:* sessions. Returns None if the user has never messaged.
pub async fn last_activity_for_user(pool: &PgPool, user_id: &str)
    -> sqlx::Result<Option<chrono::DateTime<chrono::Utc>>>;

/// Fetch messages since a timestamp, joined with sessions for user_id filtering.
/// Excludes dream:* and heartbeat:* sessions. Capped to prevent unbounded fetches.
pub async fn get_messages_since(
    pool: &PgPool, user_id: &str,
    since: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> sqlx::Result<Vec<Message>>;
```

### 3.8 Prompt templates

Ship two files in the repo:

- `plexus-server/templates/prompts/dream_phase1.md` — analysis prompt. See Task D-5 for full text.
- `plexus-server/templates/prompts/dream_phase2.md` — execution prompt. See Task D-5.

Loaded at server boot via `include_str!` with admin override from `system_config.dream_phase{1,2}_prompt` (keys already in A-20's seed list, values optional).

### 3.9 System jobs are skipped from rescheduling when they have no `next_run_at`

A-20 seeded `dream_enabled` and also the `dream_phase{1,2}_prompt` keys. A-19 set up per-channel upload caps. A-5 shipped workspace templates. Nothing in Plan A or C blocks Plan D.

## 4. File Structure

### New files

| File | Responsibility |
|---|---|
| `plexus-server/src/dream.rs` | `handle_dream_fire`, `run_phase1`, `is_dream_enabled`, plus unit tests. |
| `plexus-server/templates/prompts/dream_phase1.md` | Analysis prompt — tells the LLM how to emit structured directives. |
| `plexus-server/templates/prompts/dream_phase2.md` | Execution prompt — tells the LLM to apply directives via file tools. |

### Modified files

| File | Change |
|---|---|
| `plexus-server/src/bus.rs` | Add `EventKind` enum + `kind: EventKind` field on `InboundEvent`. Rate-limit exemption switches to kind-based. |
| `plexus-server/src/channels/gateway.rs`, `channels/discord.rs`, `channels/telegram.rs` | Set `kind: EventKind::UserTurn` on the 3 construction sites. |
| `plexus-server/src/cron.rs` | Set `kind: EventKind::Cron` on the existing event; route `kind='system' AND name='dream'` jobs to `dream::handle_dream_fire` instead of publishing. |
| `plexus-server/src/agent_loop.rs` | Add `ToolAllowlist` handling in the tool-dispatch site; route `EventKind::Dream` to `PromptMode::Dream` + dream allowlist. Update 2 test literals with `kind:`. |
| `plexus-server/src/context.rs` | Add `PromptMode` enum; `build_context` takes it. Add `PromptMode::Dream` branch. |
| `plexus-server/src/server_tools/mod.rs` | Add `ToolAllowlist` enum + `DREAM_PHASE2_ALLOWLIST` const. `ToolContext::is_cron` derives from `EventKind::Cron`. |
| `plexus-server/src/state.rs` | Add `dream_phase1_prompt: Arc<RwLock<String>>` + `dream_phase2_prompt: Arc<RwLock<String>>` fields. Boot-load these from `system_config` with `include_str!` fallback. Extend test helpers. |
| `plexus-server/src/main.rs` | `pub mod dream;`. Load dream prompts into state at boot. |
| `plexus-server/src/db/mod.rs` | Add migration `ALTER TABLE users ADD COLUMN IF NOT EXISTS last_dream_at TIMESTAMPTZ`. |
| `plexus-server/src/db/users.rs` | Add `get_last_dream_at`, `update_last_dream_at`. |
| `plexus-server/src/db/messages.rs` | Add `last_activity_for_user`, `get_messages_since`. |
| `plexus-server/src/workspace/registration.rs` | Call `ensure_system_cron_job` for dream (cron_expr `0 */2 * * *`, deliver=false). Use user's stored timezone. |

### Tests

| File | Scope |
|---|---|
| `plexus-server/src/dream.rs` inline `#[cfg(test)] mod tests` | `is_dream_enabled` parsing; directive NO-OP detection; handler idle-short-circuit path (no LLM needed — mock or test the idle-check early return directly). |
| `plexus-server/src/server_tools/mod.rs` inline | `ToolAllowlist::All.allows(x) == true`; `Only([...]).allows("foo")` matrix. |
| `plexus-server/src/context.rs` inline | `PromptMode::Dream` build_context output contains the dream system prompt and does NOT contain the channel-identity banner. |
| `plexus-server/src/workspace/registration.rs` inline (extending existing `tests` block) | A new test that checks `ensure_system_cron_job` was called for dream — can assert via a DB query when integration tests run. `#[ignore]`-gate since it needs DATABASE_URL. |
| `plexus-server/src/db/users.rs` inline | `update_last_dream_at` + `get_last_dream_at` round-trip. `#[ignore]`-gated. |
| `plexus-server/src/db/messages.rs` inline | `last_activity_for_user` returns None for empty, Some(max) for populated user excluding dream:/heartbeat: rows. `#[ignore]`-gated. |

## 5. Testing Strategy

- **Unit tests** where possible (allowlist matrix, directive parsing, PromptMode branch output shape).
- **Integration tests** gated on `DATABASE_URL` for all DB helpers and the registration flow (matches Plan C's pattern).
- **No real LLM tests.** Phase 1 / Phase 2 behavior is tested by exercising the control flow (idle short-circuit, directive-empty skip, InboundEvent publication) rather than asserting on LLM output quality.
- **Regression fence for existing tests.** 130+ existing tests must still pass. The `kind: EventKind::UserTurn` additions to channel adapters and the `mode: PromptMode::UserTurn` default for existing `build_context` callers are backward-compatible.

## 6. Tasks

10 tasks. Each ends with one commit. D-1 through D-4 are scaffolds used by D-5 onwards; do them in order.

---

### Task D-1: `EventKind` enum on `InboundEvent`

**Files:**
- Modify: `plexus-server/src/bus.rs`
- Modify: `plexus-server/src/channels/gateway.rs`
- Modify: `plexus-server/src/channels/discord.rs`
- Modify: `plexus-server/src/channels/telegram.rs`
- Modify: `plexus-server/src/cron.rs`
- Modify: `plexus-server/src/agent_loop.rs` (test literals)
- Modify: `plexus-server/src/server_tools/mod.rs` (derive `is_cron` from kind)

- [ ] **Step 1: Add the enum + field**

Edit `plexus-server/src/bus.rs`:

```rust
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EventKind {
    UserTurn,
    Cron,
    Dream,
    Heartbeat,
}

#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub session_id: String,
    pub user_id: String,
    pub kind: EventKind,
    pub content: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub media: Vec<String>,
    pub cron_job_id: Option<String>,
    pub identity: Option<crate::context::ChannelIdentity>,
}
```

- [ ] **Step 2: Update the rate-limit exemption**

Same file, `publish_inbound`:

```rust
// BEFORE: if event.cron_job_id.is_none() { check_rate_limit(...) }
// AFTER:
if event.kind == EventKind::UserTurn {
    check_rate_limit(state, &event.user_id).await?;
}
```

- [ ] **Step 3: Update all 5 construction sites**

Add `kind: EventKind::UserTurn` in channel adapters:

```rust
// plexus-server/src/channels/gateway.rs:145
let event = InboundEvent {
    session_id: ...,
    user_id: ...,
    kind: EventKind::UserTurn,    // NEW
    content: ...,
    channel: ...,
    chat_id: ...,
    media: ...,
    cron_job_id: None,
    identity: ...,
};
```

Identical addition in `channels/discord.rs` and `channels/telegram.rs` at the `InboundEvent { ... }` literals (line numbers 340 and 285 respectively).

In `cron.rs` (line 57), set `kind: EventKind::Cron`:

```rust
let event = InboundEvent {
    session_id: format!("cron:{}", job.job_id),
    user_id: job.user_id.clone(),
    kind: EventKind::Cron,        // NEW
    content: job.message.clone(),
    // ... existing fields
};
```

In `agent_loop.rs` test literals at lines 614 and 637 (the `deliver_tests` module added in C-2), add `kind: EventKind::Cron` (for line 614) and `kind: EventKind::UserTurn` (for line 637) respectively.

- [ ] **Step 4: Switch `ToolContext::is_cron` derivation**

In `agent_loop.rs` (line ~219) where `ToolContext` is built:

```rust
// BEFORE: is_cron: event.cron_job_id.is_some(),
// AFTER:
is_cron: event.kind == EventKind::Cron,
```

- [ ] **Step 5: Update the outbound-hint `InboundEvent` synthesis (if any)**

Grep once more:
```bash
grep -rn "InboundEvent {" plexus-server/src/
```
Every construction site must now include `kind:`. No implicit-default.

- [ ] **Step 6: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 130+ tests pass.

- [ ] **Step 7: Commit**

```bash
git add -u
git commit -m "feat(bus): EventKind discriminant on InboundEvent

Introduces EventKind { UserTurn | Cron | Dream | Heartbeat } as
the explicit dispatch discriminant for the message bus. Previously
cron-vs-user was inferred from cron_job_id.is_some(); that heuristic
breaks when dream (Plan D) and heartbeat (Plan E) need to travel
through the same bus but route to different prompt modes.

cron_job_id is retained — it carries the job id that
reschedule_after_completion needs. But its semantic role is now
'reschedule-handle', not 'is-cron'.

Rate-limit exemption switches to kind-based: only UserTurn events
pass through check_rate_limit; Cron/Dream/Heartbeat are server-
originated and not subject to the per-user rate limit. Same holds
for ToolContext::is_cron.

5 construction sites updated (gateway, discord, telegram, cron,
and 2 agent_loop test literals). Rate-limit behavior unchanged
for user turns; no regression.

Heartbeat variant lands now so Plan E doesn't have to bump this
file."
```

---

### Task D-2: `PromptMode` enum + `build_context` signature

**Files:**
- Modify: `plexus-server/src/context.rs`
- Modify: `plexus-server/src/agent_loop.rs` (one call site + potentially tests)
- Modify: `plexus-server/src/dream.rs` (stub the future caller — or defer this update to D-6)

- [ ] **Step 1: Add the enum**

Near the top of `context.rs`, above `build_context`:

```rust
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PromptMode {
    UserTurn,
    Dream,
    Heartbeat,
}
```

- [ ] **Step 2: Extend `build_context` signature**

Current signature (approximate — grep to confirm):
```rust
pub async fn build_context(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    // ... other params ...
) -> Result<Vec<ChatMessage>, String>;
```

New:
```rust
pub async fn build_context(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    // ... other params ...
    mode: PromptMode,
) -> Result<Vec<ChatMessage>, String>;
```

The `mode` param is threaded through to the system-prompt-assembly step. For this task, ALL branches still produce the same output (UserTurn's current behavior); the Dream/Heartbeat branches are stubs that return `UserTurn`'s result. **Task D-6 implements the real `PromptMode::Dream` branch.**

- [ ] **Step 3: Update the sole call site in `agent_loop.rs`**

Find `build_context(...)` in `handle_event`. Pass `PromptMode::UserTurn` for now:

```rust
let messages = build_context(
    state,
    user_id,
    session_id,
    // ... existing args ...
    PromptMode::UserTurn,
).await?;
```

**The real kind-based dispatch to Dream/Heartbeat lands in D-7** (dream handler) and in Plan E (heartbeat handler).

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 130+ tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(context): PromptMode enum for build_context

Adds PromptMode { UserTurn | Dream | Heartbeat } as an explicit
discriminant for build_context. UserTurn is the existing behavior.
Dream + Heartbeat return UserTurn's output for now (stubs); D-6
implements the real Dream branch, Plan E implements Heartbeat.

This lands the parameter plumbing so D-6 and Plan E don't have to
touch build_context's signature themselves."
```

---

### Task D-3: `users.last_dream_at` column + helpers

**Files:**
- Modify: `plexus-server/src/db/mod.rs`
- Modify: `plexus-server/src/db/users.rs`

- [ ] **Step 1: Schema migration**

In `db/mod.rs`, add to the migrations list (after the existing `timezone` migration):

```rust
"ALTER TABLE users ADD COLUMN IF NOT EXISTS last_dream_at TIMESTAMPTZ",
```

- [ ] **Step 2: DB helpers**

In `db/users.rs`:

```rust
pub async fn get_last_dream_at(
    pool: &PgPool,
    user_id: &str,
) -> sqlx::Result<Option<chrono::DateTime<chrono::Utc>>> {
    let row: Option<(Option<chrono::DateTime<chrono::Utc>>,)> =
        sqlx::query_as("SELECT last_dream_at FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(v,)| v))
}

pub async fn update_last_dream_at(
    pool: &PgPool,
    user_id: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET last_dream_at = $1 WHERE user_id = $2")
        .bind(at)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}
```

- [ ] **Step 3: Ignore-gated round-trip test**

In `db/users.rs`:

```rust
#[tokio::test]
#[ignore]
async fn test_last_dream_at_roundtrip() {
    let url = std::env::var("DATABASE_URL").expect("set DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    crate::db::init_db(&url).await.unwrap();

    let user_id = format!("d3-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    crate::db::users::create_user(
        &pool, &user_id, &format!("{user_id}@test.local"), "", false,
    ).await.unwrap();

    // Initially None.
    let initial = get_last_dream_at(&pool, &user_id).await.unwrap();
    assert!(initial.is_none());

    let now = chrono::Utc::now();
    update_last_dream_at(&pool, &user_id, now).await.unwrap();

    let after = get_last_dream_at(&pool, &user_id).await.unwrap().expect("set");
    // Postgres TIMESTAMPTZ has microsecond precision; compare within a ms.
    assert!((after - now).num_milliseconds().abs() < 5);

    // Cleanup
    sqlx::query("DELETE FROM users WHERE user_id = $1")
        .bind(&user_id).execute(&pool).await.ok();
}
```

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server db::users
```

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(schema): users.last_dream_at + helpers

Adds users.last_dream_at TIMESTAMPTZ column (idempotent migration)
and two helpers: get_last_dream_at returns Option<DateTime<Utc>>
(None if never dreamt), update_last_dream_at writes a new timestamp.

Plan D consumes these for the idle-check short-circuit at dream
fire time. Test is ignore-gated; run with DATABASE_URL + --ignored."
```

---

### Task D-4: `ToolAllowlist` enum + dispatch gate

**Files:**
- Modify: `plexus-server/src/server_tools/mod.rs`
- Modify: `plexus-server/src/agent_loop.rs` (tool-dispatch site)

- [ ] **Step 1: Define the enum + const**

At the top of `server_tools/mod.rs` (near `SERVER_TOOL_NAMES`):

```rust
/// Allowlist for tool dispatch. Used by restricted modes (e.g. dream phase 2)
/// to forbid tools outside a small set without touching the global registry.
#[derive(Debug, Clone)]
pub enum ToolAllowlist {
    /// Every registered tool is dispatchable.
    All,
    /// Only tools whose names appear in the slice may dispatch.
    Only(&'static [&'static str]),
}

impl ToolAllowlist {
    pub fn allows(&self, tool_name: &str) -> bool {
        match self {
            ToolAllowlist::All => true,
            ToolAllowlist::Only(names) => names.contains(&tool_name),
        }
    }
}

/// Tools available during dream Phase 2: file I/O only.
pub const DREAM_PHASE2_ALLOWLIST: &[&str] = &[
    "read_file", "write_file", "edit_file", "delete_file",
    "list_dir", "glob", "grep",
];
```

- [ ] **Step 2: Unit test the matrix**

Still in `server_tools/mod.rs`, add to any existing `#[cfg(test)] mod tests` block (or create one):

```rust
#[cfg(test)]
mod allowlist_tests {
    use super::*;

    #[test]
    fn all_allows_everything() {
        let a = ToolAllowlist::All;
        assert!(a.allows("read_file"));
        assert!(a.allows("message"));
        assert!(a.allows("anything"));
    }

    #[test]
    fn only_permits_named_tools() {
        let a = ToolAllowlist::Only(DREAM_PHASE2_ALLOWLIST);
        assert!(a.allows("read_file"));
        assert!(a.allows("write_file"));
        assert!(a.allows("grep"));
        assert!(!a.allows("message"));
        assert!(!a.allows("cron"));
        assert!(!a.allows("web_fetch"));
        assert!(!a.allows("file_transfer"));
    }
}
```

- [ ] **Step 3: Plumb the allowlist through the agent loop**

In `agent_loop.rs`, `run_session` accepts the allowlist as a parameter (or `handle_event` receives it via event-kind lookup):

```rust
pub async fn run_session(
    state: Arc<AppState>,
    session_id: String,
    user_id: String,
    mut rx: mpsc::Receiver<InboundEvent>,
) { ... }
```

The allowlist must be per-event, not per-session (different kinds in the same session use different allowlists). Easier: resolve the allowlist inside `handle_event` based on `event.kind`:

```rust
let allowlist = match event.kind {
    EventKind::Dream => ToolAllowlist::Only(DREAM_PHASE2_ALLOWLIST),
    _ => ToolAllowlist::All,
};
```

- [ ] **Step 4: Gate tool dispatch**

Find the tool-dispatch site in `handle_event`. It's the `match tc.function.name { ... }` block that routes to server_tools/client tools. Wrap with an allowlist check:

```rust
let tool_name = &tc.function.name;
let result_output = if !allowlist.allows(tool_name) {
    format!(
        "Tool '{tool_name}' is not available in this context. \
         Available tools: {}.",
        match &allowlist {
            ToolAllowlist::All => "all".to_string(),
            ToolAllowlist::Only(names) => names.join(", "),
        }
    )
} else if server_tools::is_server_tool(tool_name) {
    // existing server-tool dispatch
    server_tools::execute(state, &tool_ctx, tool_name, args).await.output
} else {
    // existing client-tool dispatch
    tools_registry::route_to_device(state, &user_id, &device_name, tool_name, args).await
};
```

Adapt to the exact existing code shape. The key invariant: BEFORE any dispatch, the allowlist must approve. On rejection, the loop records the error as the tool result and continues (the LLM sees it and may try a different approach).

- [ ] **Step 5: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server allowlist
cargo test --package plexus-server
```

Expected: allowlist tests pass; full suite passes (no regression because default kind-based resolution maps non-Dream to `All`).

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "feat(tools): ToolAllowlist enum + dispatch gate

Adds ToolAllowlist { All | Only(&'static [&'static str]) } and a
DREAM_PHASE2_ALLOWLIST constant containing the 7 file-tool names.
The agent loop resolves the allowlist per event (based on event.kind)
before dispatching each tool call. Non-Dream events default to All,
preserving existing behavior.

When an LLM attempts a tool outside the allowlist, the dispatch
records a structured error as the tool result (mentioning which
tools ARE available) and continues the loop so the LLM can recover.

Plan D's dream Phase 2 will set event.kind = Dream so the handler
binds Only(DREAM_PHASE2_ALLOWLIST). Plan E's heartbeat uses All
(full toolset)."
```

---

### Task D-5: Prompt templates + state loading

**Files:**
- Create: `plexus-server/templates/prompts/dream_phase1.md`
- Create: `plexus-server/templates/prompts/dream_phase2.md`
- Modify: `plexus-server/src/state.rs`
- Modify: `plexus-server/src/main.rs`

- [ ] **Step 1: Create the template files**

`plexus-server/templates/prompts/dream_phase1.md`:

```markdown
You are the analysis phase of a dream — a periodic memory-consolidation pass that runs while the user is away. Your job is to read the user's recent activity and decide what durable changes should be made to their long-term memory, identity, and skills. You emit structured DIRECTIVES only. A second agent applies them.

## Your inputs

You will be given:
- `## Current MEMORY.md` — the user's structured long-term memory (may be empty).
- `## Current SOUL.md` — their identity/personality as embodied by the assistant (may be empty).
- `## Skills index` — a list of `name: description` lines for each of their on-demand skills.
- `## Recent activity` — the messages that have occurred since the last dream. Read these as the source of what might need consolidation.

## Output — directives only

Emit zero or more directive lines. Do not include any prose, preamble, or explanation. Each directive is a single line (or a multi-line block for skill creation). If nothing is worth changing, emit the single line `[NO-OP]`.

### `[MEMORY-ADD] <section>\n<bullet>`

Add a bullet under the given `## section` in `MEMORY.md`. Use the existing section headers (`## User Facts`, `## Active Projects`, `## Completed`, `## Notes`). If the section is missing, the execution phase creates it.

Example:
```
[MEMORY-ADD] ## User Facts
- Prefers TypeScript over JavaScript; avoid recommending JS-first frameworks.
```

### `[MEMORY-REMOVE] <exact-text>`

Remove a line from `MEMORY.md` matching this exact text. Use for entries that have become stale or wrong.

### `[SOUL-EDIT]`

Rare. Only for identity-shaping edits the user has taught. Two lines:

```
[SOUL-EDIT]
<old exact text>
===
<new text>
```

### `[SKILL-NEW]`

Create a new skill at `skills/{name}/SKILL.md`. Format:

```
[SKILL-NEW]
name: <snake_case_name>
description: <one-line summary>
always_on: false
---
<skill body>
```

Only for patterns the user has repeated at least twice in the recent activity window. Do not create skills for one-off tasks.

### `[SKILL-DELETE] <name>`

Delete the skill directory at `skills/{name}/`. Only for skills that are clearly obsolete or duplicated.

## Rules

1. **Be parsimonious.** Emit fewer high-value directives; an empty batch is fine.
2. **Never leak secrets.** If the user shared a password, API key, or private token in chat, do NOT encode it as a memory entry.
3. **No speculative skills.** A skill represents a reusable workflow with at least 2 observed invocations in the activity window.
4. **Prefer additions.** For `MEMORY.md`, prefer `[MEMORY-ADD]` over edits unless an entry is clearly wrong.
5. **Keep section headers stable.** Do not invent new top-level sections.
6. If nothing in the activity window earns a change, respond with exactly `[NO-OP]`.
```

`plexus-server/templates/prompts/dream_phase2.md`:

```markdown
You are the execution phase of a dream. You have just received analysis DIRECTIVES as your user message. Your job is to apply them to the user's workspace by using file tools.

## Tools

You have exactly these tools: `read_file`, `write_file`, `edit_file`, `delete_file`, `list_dir`, `glob`, `grep`. All paths are relative to the user's workspace root (`MEMORY.md`, `skills/foo/SKILL.md`, etc.).

No other tools are available — no messaging, no web fetch, no file transfer.

## Workflow

1. **Read before you write.** Start with `read_file("MEMORY.md")` and `read_file("SOUL.md")` even if the directives don't seem to touch them — the section structure matters.
2. **Apply each directive as written.** Prefer `edit_file` for small surgical changes; use `write_file` for full replacements only when `edit_file` semantics don't fit.
3. **Handle errors gracefully.** If an `[MEMORY-REMOVE]` target is already gone, skip. If a `[SKILL-DELETE]` target doesn't exist, skip. Do not fail-stop the batch.
4. **For `[SKILL-NEW]`:** the `name` field is the directory name and the filename inside is always `SKILL.md`. The frontmatter MUST begin with `---\n` and end with `---\n` — match the format of existing skills.
5. **Create missing sections.** If `[MEMORY-ADD]` targets `## Active Projects` but that header doesn't exist in `MEMORY.md`, add the header before the bullet.
6. When you're done, emit a one-paragraph final message summarizing what you did. This message is NOT delivered to any channel (dream is silent); it's kept only for diagnostic logs.

## Rules

- Your workspace is scoped — you cannot read or write outside `{workspace}/{user_id}/`.
- Files are quota-checked. If you try to grow memory unboundedly the write tool will return a quota error; consolidate via `[MEMORY-REMOVE]` directives rather than piling additions.
- Do NOT invent skills that were not in the directives. Only apply what Phase 1 asked for.
```

- [ ] **Step 2: Add prompt-holder fields to `AppState`**

In `plexus-server/src/state.rs`:

```rust
pub struct AppState {
    // ... existing ...
    pub dream_phase1_prompt: Arc<tokio::sync::RwLock<String>>,
    pub dream_phase2_prompt: Arc<tokio::sync::RwLock<String>>,
}
```

Initialize with `include_str!` defaults (Plan D boot-loader will overwrite from `system_config` if present).

Update the test helpers (`test_minimal`, `test_minimal_with_quota`, `test_minimal_with_outbound`, `test_with_pool`) to set both fields with the bundled include_str!(...) defaults.

- [ ] **Step 3: Boot-load admin overrides**

In `plexus-server/src/main.rs`, after `db::system_config::seed_defaults_if_missing(&pool).await?`:

```rust
// Load admin overrides for dream prompts. Fallback to shipped templates.
let dream_phase1 = db::system_config::get(&pool, "dream_phase1_prompt")
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| {
        include_str!("../templates/prompts/dream_phase1.md").to_string()
    });
let dream_phase2 = db::system_config::get(&pool, "dream_phase2_prompt")
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| {
        include_str!("../templates/prompts/dream_phase2.md").to_string()
    });
```

Then pass both into the `AppState { ... }` literal:
```rust
dream_phase1_prompt: Arc::new(RwLock::new(dream_phase1)),
dream_phase2_prompt: Arc::new(RwLock::new(dream_phase2)),
```

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: all tests pass (nothing reads the fields yet).

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(dream): ship phase1/phase2 prompt templates + state wiring

Two prompt templates in plexus-server/templates/prompts/. Loaded at
boot from system_config.dream_phase{1,2}_prompt with include_str!
fallback — matches the admin-override pattern used for default_soul.

AppState gains dream_phase1_prompt / dream_phase2_prompt fields
behind RwLock (admin edits aren't expected often, but reads happen
per dream fire so the lock is cheap). Test helpers initialize with
the shipped defaults.

Phase 1 teaches the LLM to emit structured directives ([MEMORY-ADD],
[SKILL-NEW], etc.). Phase 2 teaches the applier to treat those
directives surgically with the file-only tool allowlist."
```

---

### Task D-6: `dream.rs` module — idle-check + Phase 1 + publish

**Files:**
- Create: `plexus-server/src/dream.rs`
- Modify: `plexus-server/src/main.rs` (`pub mod dream;`)
- Modify: `plexus-server/src/db/messages.rs` (last_activity_for_user + get_messages_since)

- [ ] **Step 1: DB helpers in `db/messages.rs`**

```rust
/// Most recent message timestamp across all non-autonomous sessions for a user.
/// Excludes 'dream:*' and 'heartbeat:*' sessions. Returns None if nothing.
pub async fn last_activity_for_user(
    pool: &PgPool,
    user_id: &str,
) -> sqlx::Result<Option<chrono::DateTime<chrono::Utc>>> {
    let row: Option<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
        "SELECT MAX(m.created_at) \
         FROM messages m \
         JOIN sessions s ON m.session_id = s.session_id \
         WHERE s.user_id = $1 \
           AND s.session_id NOT LIKE 'dream:%' \
           AND s.session_id NOT LIKE 'heartbeat:%'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(v,)| v))
}

/// Fetch the user's messages created strictly after `since`, bounded by `limit`.
/// Excludes dream: and heartbeat: sessions. Ordered ascending by created_at.
pub async fn get_messages_since(
    pool: &PgPool,
    user_id: &str,
    since: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> sqlx::Result<Vec<Message>> {
    sqlx::query_as::<_, Message>(
        "SELECT m.* FROM messages m \
         JOIN sessions s ON m.session_id = s.session_id \
         WHERE s.user_id = $1 \
           AND s.session_id NOT LIKE 'dream:%' \
           AND s.session_id NOT LIKE 'heartbeat:%' \
           AND m.created_at > $2 \
         ORDER BY m.created_at ASC \
         LIMIT $3",
    )
    .bind(user_id)
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await
}
```

- [ ] **Step 2: Create `dream.rs`**

```rust
//! Dream subsystem: periodic memory consolidation + skill discovery.
//!
//! Wired into the cron poller: when a kind='system' name='dream' job fires,
//! cron.rs dispatches to handle_dream_fire instead of publish_inbound.
//!
//! The handler does a cheap idle check (no LLM cost), and on positive
//! activity runs Phase 1 (standalone LLM call) to produce directives.
//! Non-empty directives are published as an InboundEvent { kind: Dream }
//! which agent_loop routes to PromptMode::Dream + ToolAllowlist::Only(
//! DREAM_PHASE2_ALLOWLIST) for Phase 2.

use crate::bus::{EventKind, InboundEvent};
use crate::state::AppState;
use std::sync::Arc;
use tracing::{info, warn};

const PHASE1_MESSAGE_CAP: i64 = 200;

pub async fn handle_dream_fire(
    state: &Arc<AppState>,
    job: &crate::db::cron::CronJob,
) -> Result<(), String> {
    // 1. Global kill switch.
    if !is_dream_enabled(&state.db).await {
        info!(user_id = %job.user_id, "dream: globally disabled, skipping");
        reschedule(state, &job.job_id, true).await;
        return Ok(());
    }

    // 2. Idle check.
    let last_activity = crate::db::messages::last_activity_for_user(&state.db, &job.user_id)
        .await
        .map_err(|e| format!("last_activity_for_user: {e}"))?;
    let last_dream = crate::db::users::get_last_dream_at(&state.db, &job.user_id)
        .await
        .map_err(|e| format!("get_last_dream_at: {e}"))?;

    let should_dream = match (last_activity, last_dream) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(a), Some(d)) => a > d,
    };
    if !should_dream {
        info!(user_id = %job.user_id, "dream: no new activity, skipping");
        reschedule(state, &job.job_id, true).await;
        return Ok(());
    }

    // 3. Advance last_dream_at before running phases.
    let now = chrono::Utc::now();
    if let Err(e) = crate::db::users::update_last_dream_at(&state.db, &job.user_id, now).await {
        warn!(error = %e, user_id = %job.user_id, "dream: failed to update last_dream_at, skipping");
        reschedule(state, &job.job_id, false).await;
        return Err(format!("update_last_dream_at: {e}"));
    }

    // 4. Phase 1: standalone LLM call (no tools).
    let directives = run_phase1(state, &job.user_id, last_dream).await;

    // 5. NO-OP path.
    let trimmed = directives.trim();
    if trimmed.is_empty() || trimmed == "[NO-OP]" {
        info!(user_id = %job.user_id, "dream: phase 1 emitted NO-OP, skipping phase 2");
        reschedule(state, &job.job_id, true).await;
        return Ok(());
    }

    // 6. Publish Phase 2 event. The agent loop's reschedule_after_completion
    //    will fire when this turn ends.
    let event = InboundEvent {
        session_id: format!("dream:{}", job.user_id),
        user_id: job.user_id.clone(),
        kind: EventKind::Dream,
        content: directives,
        channel: job.channel.clone(),
        chat_id: Some(job.chat_id.clone()),
        media: vec![],
        cron_job_id: Some(job.job_id.clone()),
        identity: None,
    };
    crate::bus::publish_inbound(state, event)
        .await
        .map_err(|e| format!("dream publish_inbound: {e}"))?;

    Ok(())
}

async fn is_dream_enabled(pool: &sqlx::PgPool) -> bool {
    match crate::db::system_config::get(pool, "dream_enabled").await {
        Ok(Some(v)) => v.trim() != "false",
        Ok(None) => true,  // not configured → enabled (A-20 seeds "true" but be robust)
        Err(e) => {
            warn!(error = %e, "dream: dream_enabled lookup failed, defaulting to enabled");
            true
        }
    }
}

async fn reschedule(state: &Arc<AppState>, job_id: &str, success: bool) {
    crate::cron::reschedule_after_completion(state, job_id, success).await;
}

async fn run_phase1(
    state: &Arc<AppState>,
    user_id: &str,
    last_dream: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    // Gather inputs.
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let user_root = ws_root.join(user_id);
    let memory = tokio::fs::read_to_string(user_root.join("MEMORY.md"))
        .await.unwrap_or_default();
    let soul = tokio::fs::read_to_string(user_root.join("SOUL.md"))
        .await.unwrap_or_default();

    // Skills index: name + description (no bodies — Phase 1 doesn't need them).
    let bundle = state.skills_cache.get_or_load(user_id, ws_root).await;
    let skills_index = if bundle.is_empty() {
        "(no skills yet)".to_string()
    } else {
        bundle.iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let since = last_dream.unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::UNIX_EPOCH);
    let messages = match crate::db::messages::get_messages_since(
        &state.db, user_id, since, PHASE1_MESSAGE_CAP,
    ).await {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, user_id, "dream: failed to fetch messages, skipping phase 1");
            return String::new();
        }
    };

    let activity = if messages.is_empty() {
        "(no messages in window)".to_string()
    } else {
        messages.iter().map(|m| format!("[{}] {}: {}", m.created_at, m.role, m.content))
            .collect::<Vec<_>>().join("\n")
    };

    let system_prompt = state.dream_phase1_prompt.read().await.clone();
    let user_body = format!(
        "## Current MEMORY.md\n\n{memory}\n\n## Current SOUL.md\n\n{soul}\n\n\
         ## Skills index\n\n{skills_index}\n\n## Recent activity\n\n{activity}"
    );

    let llm_config = match state.llm_config.read().await.clone() {
        Some(c) => c,
        None => {
            warn!(user_id, "dream: no LLM config, phase 1 skipped");
            return String::new();
        }
    };

    let messages = vec![
        crate::providers::openai::ChatMessage::system(system_prompt),
        crate::providers::openai::ChatMessage::user(user_body),
    ];

    match crate::providers::openai::call_llm(
        &state.http_client, &llm_config, messages, None, None,
    ).await {
        Ok(crate::providers::openai::LlmResponse::Text { content, .. }) => content,
        Ok(_) => {
            warn!(user_id, "dream: phase 1 LLM returned unexpected response shape");
            String::new()
        }
        Err(e) => {
            warn!(error = %e, user_id, "dream: phase 1 LLM call failed");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_detection() {
        // Quick sanity check on the trimming logic used in handle_dream_fire.
        let cases = vec![
            ("", true),
            ("[NO-OP]", true),
            ("\n[NO-OP]\n", true),
            ("[MEMORY-ADD] ## User Facts\n- x", false),
            ("[NO-OP] garbage after", false),  // strict equality after trim
        ];
        for (input, expected_noop) in cases {
            let trimmed = input.trim();
            let is_noop = trimmed.is_empty() || trimmed == "[NO-OP]";
            assert_eq!(is_noop, expected_noop, "input={input:?}");
        }
    }
}
```

- [ ] **Step 3: Wire `pub mod dream;` in main.rs**

- [ ] **Step 4: Build + test**

Expected: unit tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(dream): handle_dream_fire + Phase 1 LLM call

The core dream orchestrator. Invoked by the cron poller (wired in D-7)
when a kind='system' name='dream' job fires. Does the cheap idle check
first — if last_activity <= last_dream_at, skip with zero LLM cost.

On positive activity, advances last_dream_at to NOW (prevents refire
during execution), reads MEMORY.md/SOUL.md/skills index, fetches up
to 200 messages since the previous dream, and makes one direct LLM
call with the phase 1 prompt to produce structured directives.

Empty output or [NO-OP] short-circuits. Otherwise, publishes an
InboundEvent { kind: Dream, session_id: 'dream:{user_id}', content:
directives, cron_job_id: Some } onto the bus. The agent loop routes
EventKind::Dream to PromptMode::Dream + ToolAllowlist::Only(
DREAM_PHASE2_ALLOWLIST) in D-8.

Plan D's cron job has deliver=false so publish_final (C-2) silently
skips — dream never pings a channel.

Also: db::messages::last_activity_for_user + get_messages_since
helpers. Cron rescheduling for skip paths is explicit (reschedule
on the 3 no-publish branches); the publish path leaves rescheduling
to the agent-loop post-turn hook."
```

---

### Task D-7: Cron poller routes dream jobs

**Files:**
- Modify: `plexus-server/src/cron.rs`

- [ ] **Step 1: Route dream jobs**

In `poll_and_execute`, inside the `for job in claimed` loop, add the system-dream branch BEFORE the existing regular-cron publish:

```rust
for job in claimed {
    info!("Cron firing: {} [{}] kind={}", job.name, job.job_id, job.kind);

    if job.kind == crate::db::cron::SYSTEM_KIND && job.name == "dream" {
        let state = Arc::clone(state);
        let job = job.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::dream::handle_dream_fire(&state, &job).await {
                warn!(job_id = %job.job_id, "dream handler error: {e}");
                // On error, reschedule so the poller retries next cycle.
                crate::cron::reschedule_after_completion(&state, &job.job_id, false).await;
            }
        });
        continue;
    }

    // Regular cron job: existing publish path (unchanged).
    let event = InboundEvent { /* ... kind: EventKind::Cron per D-1 ... */ };
    // ...
}
```

- [ ] **Step 2: Build + test**

Expected: still 130+ tests pass. No new behavior-specific tests yet (dream's end-to-end is deferred to integration testing in later polish).

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "feat(cron): route kind=system name=dream jobs to dream handler

When the cron poller claims a dream job, dispatch to
dream::handle_dream_fire instead of publishing a regular cron
InboundEvent. The handler does its own idle check and publishing
(if warranted).

spawn'd on a new task so the poller doesn't block on the handler's
LLM call. On handler Err, the poller's task explicitly reschedules
the cron job with success=false so the usual retry cadence kicks in
next tick.

Regular kind='user' cron jobs continue to flow through the existing
publish_inbound path unchanged."
```

---

### Task D-8: `PromptMode::Dream` branch in `build_context`

**Files:**
- Modify: `plexus-server/src/context.rs`
- Modify: `plexus-server/src/agent_loop.rs` (pass kind-derived mode + allowlist)

- [ ] **Step 1: Resolve mode + allowlist from event.kind**

In `agent_loop.rs::handle_event`, derive both at the top:

```rust
let mode = match event.kind {
    EventKind::Dream => crate::context::PromptMode::Dream,
    EventKind::Heartbeat => crate::context::PromptMode::Heartbeat,
    _ => crate::context::PromptMode::UserTurn,
};
let allowlist = match event.kind {
    EventKind::Dream => crate::server_tools::ToolAllowlist::Only(
        crate::server_tools::DREAM_PHASE2_ALLOWLIST,
    ),
    _ => crate::server_tools::ToolAllowlist::All,
};
```

Pass `mode` to `build_context(...)` (D-2 already added the parameter). Apply `allowlist` at the tool-dispatch site (D-4 added the hook).

- [ ] **Step 2: Implement the PromptMode::Dream branch in `build_context`**

Inside the branching match on `mode`:

```rust
PromptMode::Dream => {
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let user_root = ws_root.join(user_id);
    let memory = tokio::fs::read_to_string(user_root.join("MEMORY.md"))
        .await.unwrap_or_default();
    let soul = tokio::fs::read_to_string(user_root.join("SOUL.md"))
        .await.unwrap_or_default();

    let bundle = state.skills_cache.get_or_load(user_id, ws_root).await;
    let skills_section = build_skills_section(&bundle);  // existing helper, if any

    let phase2 = state.dream_phase2_prompt.read().await.clone();

    format!(
        "{phase2}\n\n\
         ## Current MEMORY.md\n\n{memory}\n\n\
         ## Current SOUL.md\n\n{soul}\n\n\
         {skills_section}"
    )
}
```

If `build_skills_section` doesn't already exist as a helper, inline the equivalent loop — the existing UserTurn branch already has this logic; factor it into a shared helper if cheap.

- [ ] **Step 3: Test the mode branch**

Inline unit test in `context.rs`:

```rust
#[cfg(test)]
mod mode_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn dream_mode_system_prompt_omits_channel_identity() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("MEMORY.md"), "## User Facts\n- test").await.unwrap();
        tokio::fs::write(user_dir.join("SOUL.md"), "# Soul\nhelpful").await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        // Stub default phase2 prompt with a recognizable string.
        *state.dream_phase2_prompt.write().await = "DREAM_PHASE2_MARKER".into();

        let messages = build_context(
            &state, "alice", "dream:alice",
            /* ... other required args, use defaults ... */
            PromptMode::Dream,
        ).await.unwrap();

        let system = messages.iter().find(|m| m.role == "system").expect("system msg");
        let content = system.content.as_deref().unwrap_or_default();
        assert!(content.contains("DREAM_PHASE2_MARKER"));
        assert!(content.contains("## User Facts"));
        assert!(content.contains("SOUL.md") || content.contains("helpful"));
        // NO channel-identity banner (which UserTurn mode includes):
        assert!(!content.contains("## Channels"));
        assert!(!content.contains("Connected devices"));
    }
}
```

Adapt the `build_context` call to its real argument list.

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server context
```

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(context): PromptMode::Dream branch + agent_loop dispatch

build_context's PromptMode::Dream branch assembles a system prompt
from {workspace}/MEMORY.md + SOUL.md + skills index + the admin-
overridable dream_phase2 prompt template. It intentionally OMITS
channel identity, device list, and current time — dream is an
autonomous server-side pass, not a user-facing reply.

agent_loop::handle_event now resolves PromptMode + ToolAllowlist
from event.kind. EventKind::Dream binds PromptMode::Dream and
ToolAllowlist::Only(DREAM_PHASE2_ALLOWLIST); all other kinds
default to UserTurn + All (unchanged behavior). EventKind::Heartbeat
is a stub that defers to UserTurn until Plan E finalizes the branch."
```

---

### Task D-9: Register dream cron at user registration

**Files:**
- Modify: `plexus-server/src/workspace/registration.rs`

- [ ] **Step 1: Call `ensure_system_cron_job` after workspace init**

In `initialize_user_workspace`, after the workspace tree is seeded and before returning Ok, call C-5's helper:

```rust
// Register the dream system cron job (Plan D). Idempotent — re-running
// initialize_user_workspace for any reason won't duplicate.
if let Some(pool) = pool {
    let timezone = crate::db::users::get_timezone(pool, user_id)
        .await
        .unwrap_or_else(|_| "UTC".into());
    if let Err(e) = crate::db::cron::ensure_system_cron_job(
        pool,
        user_id,
        "dream",
        "0 */2 * * *",   // every 2 hours
        &timezone,
        "",               // message unused — dream handler bypasses the normal cron path
        "gateway",       // channel is required by the schema; unused for dream
        "-",             // chat_id likewise
        false,           // deliver=false → publish_final skips the evaluator entirely
    ).await {
        tracing::warn!(error = %e, user_id, "failed to register dream system cron job");
        // Non-fatal — workspace registration succeeded; admin can re-init to retry.
    }
}
```

The `pool: Option<&PgPool>` param was added in A-6's fix commit (`e350990`). When `pool` is None (pure-FS tests), skip cron registration.

- [ ] **Step 2: Test (ignore-gated)**

In the existing `registration.rs` test block:

```rust
#[tokio::test]
#[ignore]
async fn test_initialize_registers_dream_cron_job() {
    let url = std::env::var("DATABASE_URL").expect("set DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    crate::db::init_db(&url).await.unwrap();

    let user_id = format!("d9-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    crate::db::users::create_user(
        &pool, &user_id, &format!("{user_id}@test.local"), "", false,
    ).await.unwrap();

    let tmp = tempfile::TempDir::new().unwrap();
    initialize_user_workspace(Some(&pool), tmp.path(), &user_id).await.unwrap();

    let jobs = crate::db::cron::list_by_user(&pool, &user_id).await.unwrap();
    let dream = jobs.iter().find(|j| j.name == "dream");
    assert!(dream.is_some(), "dream cron job should be registered");
    let dream = dream.unwrap();
    assert_eq!(dream.kind, crate::db::cron::SYSTEM_KIND);
    assert!(!dream.deliver, "dream should have deliver=false");

    // Cleanup
    sqlx::query("DELETE FROM cron_jobs WHERE user_id = $1").bind(&user_id).execute(&pool).await.ok();
    sqlx::query("DELETE FROM users WHERE user_id = $1").bind(&user_id).execute(&pool).await.ok();
}
```

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "feat(workspace): register dream cron at user registration

initialize_user_workspace now calls ensure_system_cron_job for
{name: 'dream', kind: 'system', cron: '0 */2 * * *', deliver: false}.
Idempotent (C-5's helper) — re-running registration doesn't dup.

Dream's cron job uses the user's stored timezone (falls back to UTC
if the column lookup fails). Channel and chat_id are set to
placeholders ('gateway' / '-') because the dream handler bypasses
the normal publish path entirely; these fields are only constrained
by the CHECK constraint on the schema.

deliver=false ensures publish_final (C-2) short-circuits if the
agent loop ever produces a final message through dream's session.

Failure to register is non-fatal — registration succeeds so the
user can log in; admin can re-run registration to retry. An alert-
able warn log fires so the operator sees it."
```

---

### Task D-10: End-to-end smoke test documentation

**Files:**
- Modify: `plexus-server/docs/DECISIONS.md` (new ADR for the dream design)
- Modify: `plexus-server/docs/ISSUE.md` (note any lingering follow-ups)

- [ ] **Step 1: Add ADR**

Append to `plexus-server/docs/DECISIONS.md`:

```markdown
---

## ADR-40: Dream as a protected cron job with idle check

**Context:** Plexus needs nanobot-parity dream — periodic memory consolidation and skill discovery — without running expensive LLM passes on idle users.

**Options:**
- **Dedicated scheduler thread.** Simple mental model but duplicates cron's claim/dispatch/reschedule infrastructure.
- **Dream as a system-kind cron job.** Reuses all of cron's machinery (poll, claim, reschedule) by introducing a single kind='system' discriminator protected from user deletion.

**Decision:** Dream is registered as a per-user system cron job at registration (cron_expr `0 */2 * * *`, deliver=false, kind='system'). The poller detects kind='system' AND name='dream' and dispatches to `dream::handle_dream_fire` instead of publishing a regular cron event. The handler does a cheap idle check (DB-only, no LLM) before spending any Phase 1 budget.

**Outcome:** Zero LLM cost on idle users (N users × 1 DB query / 2 hours). On active users, Phase 1 emits directives that Phase 2 applies via the restricted file-tool allowlist. Dream never publishes to channels (deliver=false → publish_final skips). C-3/C-4 prevent users from deleting the dream job; C-5's ensure_system_cron_job guarantees exactly one dream job per user.
```

- [ ] **Step 2: Update ISSUE.md**

```markdown
- [ ] **Dream integration testing.** Unit tests cover the idle-check short-circuit, the allowlist matrix, and the PromptMode::Dream context-builder shape. End-to-end tests (full Phase 1 + Phase 2 against a real LLM) are deferred — they require either a mock LLM fixture or a staging environment with an API key.
- [ ] **Dream session retention / GC.** Dream sessions accumulate in the `sessions` and `messages` tables. No retention policy yet. If this becomes a DB size issue, a periodic "prune dream sessions older than N days" task is the likely answer.
```

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "docs(dream): ADR-40 + lingering follow-ups noted

ADR-40 captures the dream-as-cron-job architecture, contrasting
with the alternative dedicated-scheduler design. Key decision
points are the idle-check location (poller vs agent loop) and
the reuse of cron's reschedule machinery.

ISSUE.md adds two deferred items: end-to-end dream integration
tests (need a mock LLM or staging) and dream-session retention
(no policy yet, defer until DB size becomes a real concern)."
```

---

## 7. Self-Review Checklist (run before declaring Plan D done)

1. **Spec coverage** against §8 of the spec:
   - §8.1 Purpose → covered by D-6's handle_dream_fire.
   - §8.2 Cron-job architecture → D-7 wires it; D-9 registers the job.
   - §8.3 Idle check at fire time → D-6's `should_dream` logic.
   - §8.4 Data model → D-3 adds `last_dream_at`; D-9 uses `cron_jobs.kind` from A-2.
   - §8.5 Phase 1 analysis → D-6's `run_phase1`.
   - §8.6 Phase 2 execution → D-8's agent-loop branch.
   - §8.7 Failure handling → D-6 advances last_dream_at BEFORE phases (prevents refire loop).
   - §8.8 Session retention → deferred (D-10 documents).

2. **Cross-cutting scaffolding** from §11:
   - §11.1 EventKind → D-1.
   - §11.2 PromptMode → D-2 + D-8.
   - §11.5 Prompt templates → D-5.

3. **Placeholder scan:** search for "TBD" / "TODO" / "similar to" — should be absent or explicitly deferred to Plan E.

4. **Type consistency:**
   - `EventKind` variants exactly `UserTurn | Cron | Dream | Heartbeat` across all uses.
   - `PromptMode` variants exactly `UserTurn | Dream | Heartbeat`.
   - `ToolAllowlist::Only(&'static [&'static str])` — `DREAM_PHASE2_ALLOWLIST` matches.
   - `handle_dream_fire(state, job)` signature matches the cron-poller's dispatch.

5. **Plan A alignment:** Dream uses the workspace model for MEMORY.md/SOUL.md/skills. Good — Plan A's context-builder reads already lean on these files.

6. **Plan C alignment:** `ensure_system_cron_job` called from D-9; `publish_final`'s deliver=false branch handles dream's silent post-run; `SYSTEM_KIND` const reused in the cron-poller dispatch.

## 8. Execution Hints

- Tasks D-1 through D-5 are scaffold; do them in order (D-1 must land before D-2 because PromptMode's test might cross paths, and D-6 depends on all four).
- D-6 is the single biggest task; plan for one larger review cycle.
- D-7 and D-8 are small once D-4/D-6 land.
- D-9 depends only on C-5 (which is already committed).
- D-10 is pure docs.
- End-to-end dream integration testing (`dream_integration.rs`) is **deferred** — without a mock LLM layer, exercising Phase 1/Phase 2 requires a staging environment. Unit-test coverage focuses on the control flow (idle short-circuit, directive empty→skip, publishment happens only on non-empty directives).
- If a subagent hits a surprising shape (e.g., `build_context`'s arg list is longer than expected), ask to re-scope before inventing — the integration points matter.
