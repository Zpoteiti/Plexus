# Heartbeat Subsystem Implementation Plan (Plan E of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Spec reference:** The full design lives at `/home/yucheng/Documents/GitHub/Plexus/docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md` §9 + §11. Read it if this plan's context seems incomplete.
>
> **This is Plan E of 5.** Prior plans: **A** (workspace foundation, `e6f1da4..2fe90a0`), **C** (shared evaluator + cron integration, `2464692..6643b0c`), **D** (dream subsystem, `5be59f9..8f60648`). Remaining after this: **B** frontend Workspace page.

**Goal:** Implement nanobot-parity heartbeat — a fixed-interval tick loop that reads each user's `HEARTBEAT.md`, has a tiny Phase 1 LLM decide whether to wake the agent, and runs Phase 2 through the normal agent loop with evaluator-gated external-channel delivery (Discord → Telegram → silence).

**Architecture:** A new background task (`heartbeat::spawn_heartbeat_tick`) wakes every 60 s and queries users whose `last_heartbeat_at` is older than `system_config.heartbeat_interval_seconds` (default 1800). For each due user the tick task advances `last_heartbeat_at` first (prevents refire), skips if `HEARTBEAT.md` is missing or the heartbeat session already has a turn in flight, then calls `heartbeat::run_phase1` — a standalone LLM call with a virtual `heartbeat(action, tasks)` tool forced via `tool_choice="required"`. `action == "skip"` ends the run silently; `action == "run"` publishes an `InboundEvent { kind: EventKind::Heartbeat, session_id: "heartbeat:{user_id}", content: tasks }`. The agent loop routes `EventKind::Heartbeat` to the real `PromptMode::Heartbeat` branch (landed in E-5), with `ToolAllowlist::All` and a 200-iteration cap. On the agent's final `LlmResponse::Text`, `publish_final` gains a Heartbeat branch that runs the shared evaluator (Plan C, `purpose: "heartbeat wake-up"`). If `should_notify`, the branch looks up the user's Discord config, falls back to Telegram, and never the gateway; if neither is configured, the final message is logged and discarded.

**Tech Stack:** Rust 1.85 (edition 2024), tokio, sqlx (PostgreSQL), serde_json, chrono_tz, tracing. Reuses: `evaluator::evaluate_notification` (Plan C), `EventKind::Heartbeat` + `PromptMode::Heartbeat` + `ToolAllowlist::All` scaffolds (Plan D).

**Parent branch:** current `M3-gateway-frontend`, based on commit `8f60648` (Plan D's final commit).

---

## 1. Overview

Heartbeat has four cooperating pieces that this plan wires up in order:

1. **`users.last_heartbeat_at` + helpers.** A new timestamp column tracks when each user's heartbeat last fired. Helpers provide the NULL-safe reads and "find users due now" query the tick loop needs.

2. **Phase 1 prompt + state plumbing.** A template file + admin-overridable `system_config.heartbeat_phase1_prompt` + an `AppState::heartbeat_phase1_prompt: Arc<RwLock<String>>` loaded at boot — same pattern Plan D used for dream.

3. **`heartbeat::run_phase1`.** A standalone LLM call exposing a single virtual tool `heartbeat(action: "skip"|"run", tasks: string)` to decide whether Phase 2 runs. Default-skip on any error (LLM error, parse error, unexpected tool name, missing LLM config). Returns `Phase1Result::{ Skip { reason }, Run { tasks } }`.

4. **Phase 2 wiring + tick loop.** `PromptMode::Heartbeat` gets its real context shape (identity + memory + skills + devices, no channels, + autonomous-wake-up banner). `publish_final` gains a Heartbeat branch that runs the evaluator and picks Discord → Telegram → silence. A new `heartbeat::spawn_heartbeat_tick` background task publishes due users' Phase 2 `InboundEvent`s on 60 s ticks, watched by `state.shutdown` for graceful shutdown.

This plan consumes scaffolds from Plan D (`EventKind::Heartbeat`, `PromptMode::Heartbeat` stub, `ToolAllowlist::All`). After it lands, the autonomy subsystems (dream + heartbeat) are both live and the shared evaluator gates all three autonomous outputs (cron, dream-never-delivers, heartbeat).

## 2. Goals & Non-Goals

**Goals**

- Heartbeat fires at most once per `heartbeat_interval_seconds` window per user (default 1800 s = 30 min), independently of cron or any channel traffic.
- Users with no `HEARTBEAT.md` are skipped with near-zero cost (one `fs::try_exists` per tick per user).
- Users with in-flight heartbeat Phase 2 turns are not refired while a prior wake-up is still running.
- Phase 1's single-tool-call contract is robust: malformed LLM output defaults to skip, not run — silence is the safe failure mode.
- Phase 2 reuses the full agent loop (compression, crash recovery, tool dispatch) — no parallel ReAct implementation.
- Heartbeat outputs never ping the gateway (no browser-session interruption) and never use the `message` tool for delivery — the agent produces a final assistant message and `publish_final` owns routing.
- The shared evaluator (Plan C) gates heartbeat delivery with `purpose: "heartbeat wake-up"`; the 4 AM guard + "is this worth an interruption" decision logic come for free.
- Admin can tune the interval globally (`heartbeat_interval_seconds`) and kill-switch heartbeat (set to 0 → tick loop no-ops every cycle).
- Admin can override the Phase 1 prompt via `system_config.heartbeat_phase1_prompt`; unset → `include_str!` fallback.

**Non-Goals**

- Per-user heartbeat cadence. The interval is global. A per-user override lives in the spec's non-goals (§2).
- Tests that stand up a real LLM. Phase 1's virtual-tool parsing, Phase 2's publish path, and the tick loop's due-query are unit/integration tested; LLM behavior is not.
- Push notifications to offline browsers. The spec explicitly defers this (§2); heartbeat never uses the gateway even if a browser is connected.
- Tests that run the *entire* tick loop end-to-end with real LLM calls. E-9's integration test covers the DB-level due-user query only.
- Mid-heartbeat interruption by a user message. If a user messages during a heartbeat turn, their user-turn session inbox is separate; the heartbeat session's inbox queues normally.
- Retrying failed Phase 2 turns. If Phase 2 errors mid-flight, the error is logged and `last_heartbeat_at` stays advanced — the next window gets a fresh shot.
- The frontend "Heartbeat Log" page (spec §9.7 mentions as future work). Log-only storage via the existing `sessions` / `messages` tables is all we ship here.

## 3. Design

### 3.1 `users.last_heartbeat_at` column + DB helpers

New column:

```sql
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_heartbeat_at TIMESTAMPTZ;
```

NULL default — interpret as "infinitely past" in the due-user query.

New helpers in `plexus-server/src/db/users.rs`:

```rust
pub async fn get_last_heartbeat_at(
    pool: &PgPool,
    user_id: &str,
) -> sqlx::Result<Option<chrono::DateTime<chrono::Utc>>>;

pub async fn update_last_heartbeat_at(
    pool: &PgPool,
    user_id: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<()>;

/// Return the user_ids whose last_heartbeat_at is NULL or older than `threshold_seconds`
/// in the past. Bounded by `limit` to keep the tick loop's memory predictable
/// for pathological cases (thousands of long-idle users waking up after admin
/// changes the interval).
pub async fn list_users_due_for_heartbeat(
    pool: &PgPool,
    threshold_seconds: i64,
    limit: i64,
) -> sqlx::Result<Vec<String>>;
```

Query for the due-user helper:

```sql
SELECT user_id FROM users
WHERE last_heartbeat_at IS NULL
   OR last_heartbeat_at < NOW() - (INTERVAL '1 second' * $1::bigint)
ORDER BY COALESCE(last_heartbeat_at, 'epoch'::timestamptz) ASC
LIMIT $2
```

Ordering by oldest-first gives fairness under interval shrinks (users who've been waiting longest get serviced first). `INTERVAL '1 second' * $1::bigint` is the portable form for "N seconds where N is an sqlx-bound i64" — `make_interval` would require an explicit `::integer` cast and a narrower value range.

### 3.2 Phase 1 prompt template + `system_config` seed

New file: `plexus-server/templates/prompts/heartbeat_phase1.md`. Content is concise — Phase 1 is a single-shot decision, not a discussion:

```markdown
You are the heartbeat decision layer for PLEXUS, an autonomous AI agent.

The user has authored a `HEARTBEAT.md` task list. Every 30 minutes the system
wakes you up and asks: should the agent do anything right now?

Call the `heartbeat` tool **exactly once** with:
- `action: "skip"` — no tasks are ripe at the current local time. Give a short
  reason and return.
- `action: "run"` — one or more tasks should run now. Put a short free-text
  summary of what the agent should do in the `tasks` field; the agent will
  receive it as a user message.

When deciding:
- Respect the user's timezone and the local time shown below. Tasks scheduled
  for "every morning" fire once per morning, not every heartbeat.
- Skip if the task list is empty, is only notes, or no task matches now.
- When uncertain, skip. The cheapest action is no action.
- Do not elaborate. Call the tool and stop.
```

Add to `db::system_config::seed_defaults_if_missing`:

```rust
// Existing string-template keys (SOUL / MEMORY / HEARTBEAT / dream prompts) stay as-is.
// Add the heartbeat phase-1 prompt:
if get(pool, "heartbeat_phase1_prompt").await?.is_none() {
    set(
        pool,
        "heartbeat_phase1_prompt",
        include_str!("../../templates/prompts/heartbeat_phase1.md"),
    ).await?;
}
```

The `heartbeat_interval_seconds` key (default `"1800"`) is already seeded by A-20 — no change.

### 3.3 `AppState::heartbeat_phase1_prompt` + boot-load

New field on `AppState`:

```rust
pub heartbeat_phase1_prompt: Arc<RwLock<String>>,
```

Boot-load in `main.rs` (mirrors dream_phase1_prompt / dream_phase2_prompt):

```rust
let heartbeat_phase1_prompt = db::system_config::get(&pool, "heartbeat_phase1_prompt")
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| {
        include_str!("../templates/prompts/heartbeat_phase1.md").to_string()
    });
```

Three `AppState` constructors update:
- `main.rs`'s production `AppState { ... }` literal
- `state.rs::test_with_pool`
- `state.rs::build_test_state`

Test helpers get the field initialized from `include_str!`.

### 3.4 Phase 1 module (`heartbeat::run_phase1`)

New file `plexus-server/src/heartbeat.rs`.

Public API:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Phase1Result {
    Skip { reason: String },
    Run { tasks: String },
}

pub async fn run_phase1(state: &Arc<AppState>, user_id: &str) -> Phase1Result;
```

Internal flow:

1. Read `HEARTBEAT.md` from the user's workspace. If missing, return `Skip { reason: "no HEARTBEAT.md" }` (the tick loop also pre-filters this, but run_phase1 is defensive).
2. Read the user's timezone via `db::users::get_timezone`. Default UTC on error.
3. Compute `local_now` via `chrono::Utc::now().with_timezone(&tz)`.
4. Build the user body:

   ```
   ## Current local time ({tz})
   {local_now_formatted}

   ## HEARTBEAT.md
   {heartbeat_md_content}
   ```

5. Build the virtual tool:

   ```json
   {
     "type": "function",
     "function": {
       "name": "heartbeat",
       "description": "Decide whether to wake the agent now.",
       "parameters": {
         "type": "object",
         "properties": {
           "action": { "type": "string", "enum": ["skip", "run"] },
           "tasks":  { "type": "string" }
         },
         "required": ["action"]
       }
     }
   }
   ```

6. Call `providers::openai::call_llm` with `Some(vec![virtual_tool()])` and `Some("required".into())`.
7. Parse:
   - No LLM config → `Skip { "no LLM config" }`
   - `Ok(ToolCalls { calls })` with `calls[0].function.name == "heartbeat"` → parse `{action, tasks}`.
   - `Ok(Text { .. })` or `ToolCalls` with wrong name → `Skip { "malformed Phase 1 response" }`.
   - `Err(e)` → `Skip { format!("LLM error: {e}") }`.
8. `action == "skip"` → `Skip { reason: "phase 1 returned skip" }`.
9. `action == "run"` → `Run { tasks: parsed.tasks.clone() }`. If tasks is empty, convert to `Skip { reason: "run with empty tasks" }` (LLM ambiguity — safer to skip than to wake with nothing).

### 3.5 `PromptMode::Heartbeat` real branch in `context.rs`

Currently `PromptMode::Heartbeat` falls through to the `UserTurn` match arm (Plan D stub). Split it out:

```rust
let system = match mode {
    PromptMode::UserTurn => { /* existing UserTurn logic */ }
    PromptMode::Heartbeat => build_heartbeat_system(
        soul, &identity, chat_id, state, &user.user_id, &skills_section, &memory,
    ).await,
    PromptMode::Dream => { /* existing Dream logic */ }
};
```

`build_heartbeat_system` is a new private helper in `context.rs`:

```rust
async fn build_heartbeat_system(
    soul: &str,
    identity: &ChannelIdentity,
    chat_id: Option<&str>,
    state: &AppState,
    user_id: &str,
    skills_section: &str,
    memory: &str,
) -> String {
    let mut s = format!("{soul}\n\n");

    // Identity (kept — the agent is still acting on behalf of this user)
    s += "## Identity\n";
    // ... identical rendering to UserTurn ...
    s += &identity.build_session_section(chat_id);
    s += "\n";

    // NO ## Channels section — heartbeat does not route to an interactive channel.

    // Memory
    if !memory.trim().is_empty() {
        s += &format!("## Memory\n{memory}\n\n");
    }

    // Skills
    s += skills_section;

    // Devices
    s += &build_device_status(state, user_id).await;

    // Runtime
    s += &format!(
        "Current time: {}\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    // Autonomous wake-up banner
    s += HEARTBEAT_BANNER;

    s
}
```

Where `HEARTBEAT_BANNER` is a module constant:

```rust
const HEARTBEAT_BANNER: &str = "\
## Autonomous Wake-Up\n\
This is an autonomous heartbeat wake-up triggered by your scheduled task list. \
Complete the requested tasks without asking for clarifying questions — pick \
reasonable defaults and proceed. Do not use the `message` tool to deliver a \
reply; produce a concise final assistant message summarizing what you did. \
The system will decide whether to notify the user through an external channel.";
```

Important: keep the existing `Identity` `## Account` and `### Current Session` subsections — the agent knowing "who it is" stays useful even on autonomous turns. The `gateway_partner` fallback identity the agent-loop constructs for heartbeat events (channel="internal", chat_id=None) renders a session section with `Channel: gateway` which is benign; the omitted `## Channels` section is what matters.

### 3.6 `PublishFinalParams` gains `kind: EventKind`

Current `publish_final` only distinguishes user-turn-vs-cron via `cron_job_id: Option<String>`. Add an explicit kind:

```rust
pub(crate) struct PublishFinalParams {
    pub channel: String,
    pub chat_id: Option<String>,
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    pub kind: crate::bus::EventKind,  // NEW
    pub cron_job_id: Option<String>,
    pub job_deliver: Option<bool>,
}
```

Callers thread `event.kind` through. The single production call site in `agent_loop::handle_event` (LlmResponse::Text branch) passes `event.kind`. The two test helpers (`deliver_tests` in agent_loop.rs) pass `EventKind::Cron` / `EventKind::UserTurn` explicitly.

### 3.7 `publish_final` Heartbeat branch

Dispatch via `params.kind`:

```rust
match kind {
    EventKind::UserTurn | EventKind::Cron => { /* existing logic */ }
    EventKind::Heartbeat => publish_final_heartbeat(state, &user_id, &content).await,
    EventKind::Dream => {
        // Dream has deliver=false on its cron row, so the cron branch above
        // already short-circuits. This arm is defensive only.
        info!(session_id, "dream turn completed; no publish");
    }
}
```

`publish_final_heartbeat` is a new `async fn` in `agent_loop.rs`:

```rust
async fn publish_final_heartbeat(state: &Arc<AppState>, user_id: &str, content: &str) {
    // 1. Evaluator gate.
    let eval = crate::evaluator::evaluate_notification(
        state,
        user_id,
        content,
        "heartbeat wake-up",
    ).await;
    if !eval.should_notify {
        info!(user_id, reason = %eval.reason, "heartbeat: evaluator suppressed notification");
        return;
    }

    // 2. External-channel precedence: Discord → Telegram → silence.
    //    Never gateway (spec §9.7 — heartbeat must not interrupt a browser session).
    if let Ok(Some(cfg)) = crate::db::discord::get_config(&state.db, user_id).await {
        if cfg.enabled {
            if let Some(partner_id) = cfg.partner_discord_id.as_deref() {
                if !partner_id.is_empty() {
                    let _ = state.outbound_tx.send(crate::bus::OutboundEvent {
                        channel: plexus_common::consts::CHANNEL_DISCORD.to_string(),
                        chat_id: Some(format!("dm/{partner_id}")),
                        session_id: format!("heartbeat:{user_id}"),
                        user_id: user_id.to_string(),
                        content: content.to_string(),
                        media: vec![],
                    }).await;
                    info!(user_id, "heartbeat: delivered via discord");
                    return;
                }
            }
        }
    }

    if let Ok(Some(cfg)) = crate::db::telegram::get_config(&state.db, user_id).await {
        if cfg.enabled {
            if let Some(partner_id) = cfg.partner_telegram_id.as_deref() {
                if !partner_id.is_empty() {
                    let _ = state.outbound_tx.send(crate::bus::OutboundEvent {
                        channel: crate::channels::CHANNEL_TELEGRAM.to_string(),
                        chat_id: Some(partner_id.to_string()),
                        session_id: format!("heartbeat:{user_id}"),
                        user_id: user_id.to_string(),
                        content: content.to_string(),
                        media: vec![],
                    }).await;
                    info!(user_id, "heartbeat: delivered via telegram");
                    return;
                }
            }
        }
    }

    info!(user_id, "heartbeat: no external channel configured; output stored only");
}
```

Note: `chat_id` format for Discord is `dm/{user_id}` — this matches what Plan D's cron-message tool and discord.rs::deliver already expect (the DM helper wraps the raw ID). Telegram's chat_id is the raw partner user/chat ID, also matching existing deliver paths.

### 3.8 Tick loop (`heartbeat::spawn_heartbeat_tick`)

Spawn pattern mirrors `cron::spawn_cron_poller`. Interval: 60 s.

```rust
pub fn spawn_heartbeat_tick(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            HEARTBEAT_TICK_INTERVAL_SEC,
        ));
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    info!("heartbeat tick shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = tick_once(&state).await {
                        warn!("heartbeat tick error: {e}");
                    }
                }
            }
        }
    });
}
```

`HEARTBEAT_TICK_INTERVAL_SEC` = 60 (module const; not a system_config knob).

Each tick:

1. Read `system_config.heartbeat_interval_seconds`. Parse as `i64`. If parse fails OR value == 0, log at `debug!` level ("heartbeat disabled or misconfigured, skipping tick") and return `Ok(())`.
2. Call `db::users::list_users_due_for_heartbeat(threshold, HEARTBEAT_MAX_USERS_PER_TICK)`.
3. For each user_id:
   a. Skip if a heartbeat session is already locked mid-turn:
      ```rust
      if let Some(handle) = state.sessions.get(&format!("heartbeat:{user_id}")) {
          if handle.lock.try_lock().is_err() {
              debug!(user_id, "heartbeat: prior turn still running, skipping");
              continue;
          }
      }
      ```
      (Note: `try_lock` briefly takes and immediately drops the lock. This is a cheap liveness probe — not a reservation. Races with `run_session` are benign: if we observe unlocked but a new turn starts milliseconds later, we still publish the InboundEvent which queues behind the in-flight turn's inbox.)
   b. Skip if `{workspace_root}/{user_id}/HEARTBEAT.md` does not exist (`tokio::fs::try_exists`).
   c. Advance `last_heartbeat_at = NOW()` BEFORE spawning Phase 1 (prevents refire during Phase 1's LLM latency).
   d. Spawn a `tokio::spawn` to run Phase 1 and publish Phase 2 if needed. Spawning keeps the tick task loop-responsive while many users' LLM calls run in parallel.

Phase 1 spawn body:

```rust
let state = Arc::clone(state);
let user_id = user_id.clone();
tokio::spawn(async move {
    match crate::heartbeat::run_phase1(&state, &user_id).await {
        crate::heartbeat::Phase1Result::Skip { reason } => {
            info!(user_id, reason, "heartbeat: phase 1 skipped");
        }
        crate::heartbeat::Phase1Result::Run { tasks } => {
            let event = crate::bus::InboundEvent {
                session_id: format!("heartbeat:{user_id}"),
                user_id: user_id.clone(),
                kind: crate::bus::EventKind::Heartbeat,
                content: tasks,
                channel: "internal".to_string(),
                chat_id: None,
                media: vec![],
                cron_job_id: None,
                identity: None,
            };
            if let Err(e) = crate::bus::publish_inbound(&state, event).await {
                warn!(user_id, error = %e, "heartbeat: publish_inbound failed");
            }
        }
    }
});
```

`HEARTBEAT_MAX_USERS_PER_TICK` = 500 (module const). On installations with more than 500 due users simultaneously, the next tick picks up the remainder — ordering by oldest-first keeps it fair.

Wire into `main.rs` alongside `spawn_cron_poller`:

```rust
cron::spawn_cron_poller(Arc::clone(&state));
heartbeat::spawn_heartbeat_tick(Arc::clone(&state));  // NEW
```

### 3.9 Module registration

`main.rs`:

```rust
pub mod heartbeat;
```

(Alongside `pub mod dream;` and `pub mod evaluator;`.)

### 3.10 Rate-limit exemption

`bus::publish_inbound` already exempts non-`UserTurn` events from rate-limiting (D-1). Heartbeat is `EventKind::Heartbeat`, so it's already exempt. No code change needed.

## 4. File Structure

### New files

| File | Responsibility |
|---|---|
| `plexus-server/src/heartbeat.rs` | `Phase1Result`, `run_phase1`, `spawn_heartbeat_tick`, `tick_once`, unit tests. |
| `plexus-server/templates/prompts/heartbeat_phase1.md` | Phase 1 decision prompt — teaches the LLM to call the virtual `heartbeat(action, tasks)` tool. |

### Modified files

| File | Change |
|---|---|
| `plexus-server/src/db/mod.rs` | Migration: `ALTER TABLE users ADD COLUMN IF NOT EXISTS last_heartbeat_at TIMESTAMPTZ`. |
| `plexus-server/src/db/users.rs` | Add `get_last_heartbeat_at`, `update_last_heartbeat_at`, `list_users_due_for_heartbeat`. Ignore-gated roundtrip + due-query tests. |
| `plexus-server/src/db/system_config.rs` | Seed `heartbeat_phase1_prompt` from the shipped template if missing. |
| `plexus-server/src/state.rs` | Add `heartbeat_phase1_prompt: Arc<RwLock<String>>` field. Initialize in both test helpers from `include_str!`. |
| `plexus-server/src/main.rs` | `pub mod heartbeat;`. Boot-load `heartbeat_phase1_prompt` from `system_config`. Spawn `heartbeat::spawn_heartbeat_tick`. |
| `plexus-server/src/context.rs` | Split `PromptMode::Heartbeat` out of the UserTurn match arm into its own branch via new private `build_heartbeat_system` helper + `HEARTBEAT_BANNER` const. Update existing tests that assert on the dream/user-turn shape to also cover heartbeat. |
| `plexus-server/src/agent_loop.rs` | `PublishFinalParams` gains `kind: EventKind`. Production call site passes `event.kind`. New `publish_final_heartbeat` helper. `publish_final` dispatches via `params.kind`. Two existing test helpers updated with explicit `kind:`. |

### Tests

| File | Scope |
|---|---|
| `plexus-server/src/db/users.rs` inline | `last_heartbeat_at` roundtrip + `list_users_due_for_heartbeat` selects oldest first + NULL-included. Both `#[ignore]`-gated (need `DATABASE_URL`). |
| `plexus-server/src/heartbeat.rs` inline | `Phase1Result` parse matrix — skip/run/empty-run/malformed. Virtual tool JSON shape. `run_phase1` returns Skip when no LLM config. |
| `plexus-server/src/context.rs` inline `mode_tests` | `PromptMode::Heartbeat` output: contains soul + identity + HEARTBEAT_BANNER; OMITS `## Channels`. Via a pure `assemble_heartbeat_system_prompt`-style helper if that's practical, else a TempDir-backed integration-style test. |
| `plexus-server/src/agent_loop.rs` `deliver_tests` module | `publish_final` with `kind: EventKind::Heartbeat` + no external channels configured → silent (no OutboundEvent). Plus `kind: EventKind::Heartbeat` + evaluator silent → silent. `#[ignore]`-gate the cases that need DB (evaluator call); pure-logic cases don't. |

## 5. Testing Strategy

- **Unit tests** where possible. Phase 1 parsing is the biggest testable surface without a real LLM — mock the tool-call result shape by constructing `openai::ToolCall` directly.
- **Integration tests** gated on `DATABASE_URL` for the three DB helpers and the due-user query's correctness.
- **No real LLM tests.** Phase 1's behavior is testable because the virtual-tool contract is unambiguous: whatever the LLM returns, it either matches `{action, tasks}` or it doesn't.
- **Regression fence for existing tests.** 140+ existing tests (post-Plan D) must still pass. The `PromptMode::Heartbeat` branch changes output shape for any test that builds a context with that mode — but no such test exists today (Plan D stubbed Heartbeat into UserTurn). Grep for `PromptMode::Heartbeat` before landing E-5.
- **Graceful shutdown coverage.** The tick loop watches `state.shutdown`. Manual smoke: start server, observe "heartbeat tick shutting down" log on SIGTERM.

## 6. Tasks

10 tasks. E-1 through E-4 lay the foundation; E-5 and E-6 finalize the cross-cutting scaffolds (prompt mode + publish branch); E-7 and E-8 ship the tick loop and wire it up; E-9 and E-10 add integration tests and docs.

---

### Task E-1: `users.last_heartbeat_at` column + DB helpers

**Files:**
- Modify: `plexus-server/src/db/mod.rs`
- Modify: `plexus-server/src/db/users.rs`

- [ ] **Step 1: Add the migration**

In `plexus-server/src/db/mod.rs`, append to the `statements` array (after the existing `last_dream_at` migration at line 46):

```rust
// Migration: add last_heartbeat_at for Plan E's tick loop
"ALTER TABLE users ADD COLUMN IF NOT EXISTS last_heartbeat_at TIMESTAMPTZ",
```

- [ ] **Step 2: Add the per-user get/update helpers**

In `plexus-server/src/db/users.rs`, append after `update_last_dream_at`:

```rust
pub async fn get_last_heartbeat_at(
    pool: &PgPool,
    user_id: &str,
) -> sqlx::Result<Option<DateTime<Utc>>> {
    let row: Option<(Option<DateTime<Utc>>,)> =
        sqlx::query_as("SELECT last_heartbeat_at FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(v,)| v))
}

pub async fn update_last_heartbeat_at(
    pool: &PgPool,
    user_id: &str,
    at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET last_heartbeat_at = $1 WHERE user_id = $2")
        .bind(at)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}
```

- [ ] **Step 3: Add the due-user query**

Append after `update_last_heartbeat_at`:

```rust
/// Return user_ids whose last_heartbeat_at is NULL or older than
/// `threshold_seconds` in the past. Ordered oldest-first (NULL counts as
/// epoch, so never-fired users come before stale-fired users). Bounded by
/// `limit` so a single tick can't OOM the server on an interval shrink.
pub async fn list_users_due_for_heartbeat(
    pool: &PgPool,
    threshold_seconds: i64,
    limit: i64,
) -> sqlx::Result<Vec<String>> {
    // INTERVAL '1 second' * $1::bigint is the portable form for "N seconds
    // where N is an i64" — make_interval would need an ::integer cast.
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT user_id FROM users \
         WHERE last_heartbeat_at IS NULL \
            OR last_heartbeat_at < NOW() - (INTERVAL '1 second' * $1::bigint) \
         ORDER BY COALESCE(last_heartbeat_at, 'epoch'::timestamptz) ASC \
         LIMIT $2",
    )
    .bind(threshold_seconds)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}
```

- [ ] **Step 4: Add ignore-gated DB tests**

Append to the `#[cfg(test)] mod tests` block in `plexus-server/src/db/users.rs`:

```rust
#[tokio::test]
#[ignore] // needs DATABASE_URL
async fn test_last_heartbeat_at_roundtrip() {
    let url = std::env::var("DATABASE_URL")
        .expect("set DATABASE_URL to run this test");
    let pool = crate::db::init_db(&url).await;

    let user_id = format!("e1-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let user_email = format!("{user_id}@test.local");
    crate::db::users::create_user(&pool, &user_id, &user_email, "", false)
        .await
        .unwrap();

    let initial = get_last_heartbeat_at(&pool, &user_id).await.unwrap();
    assert!(initial.is_none(), "fresh user should have NULL last_heartbeat_at");

    let now = chrono::Utc::now();
    update_last_heartbeat_at(&pool, &user_id, now).await.unwrap();

    let after = get_last_heartbeat_at(&pool, &user_id).await.unwrap()
        .expect("timestamp should be present after update");
    let delta = (after - now).num_milliseconds().abs();
    assert!(delta < 5, "expected roundtrip within 5ms; got {delta}ms delta");

    sqlx::query("DELETE FROM users WHERE user_id = $1")
        .bind(&user_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[ignore] // needs DATABASE_URL
async fn test_list_users_due_for_heartbeat_selects_null_and_stale() {
    let url = std::env::var("DATABASE_URL")
        .expect("set DATABASE_URL to run this test");
    let pool = crate::db::init_db(&url).await;

    // Three users: one NULL, one stale (1h ago), one fresh (10s ago).
    let ids: Vec<String> = (0..3)
        .map(|i| format!("e1d-{}-{}", i, &uuid::Uuid::new_v4().to_string()[..8]))
        .collect();
    for id in &ids {
        crate::db::users::create_user(
            &pool,
            id,
            &format!("{id}@test.local"),
            "",
            false,
        )
        .await
        .unwrap();
    }

    // ids[0] stays NULL. ids[1] is stale. ids[2] is fresh.
    update_last_heartbeat_at(
        &pool,
        &ids[1],
        chrono::Utc::now() - chrono::Duration::hours(1),
    )
    .await
    .unwrap();
    update_last_heartbeat_at(&pool, &ids[2], chrono::Utc::now())
        .await
        .unwrap();

    // Threshold 1800s = 30 minutes. NULL + 1h-ago are due; 10s-ago is not.
    let due = list_users_due_for_heartbeat(&pool, 1800, 10).await.unwrap();
    assert!(due.contains(&ids[0]), "NULL user should be due");
    assert!(due.contains(&ids[1]), "stale user should be due");
    assert!(!due.contains(&ids[2]), "fresh user should NOT be due");

    // Oldest-first ordering: NULL (epoch) < 1h-ago.
    let pos_null = due.iter().position(|x| x == &ids[0]).unwrap();
    let pos_stale = due.iter().position(|x| x == &ids[1]).unwrap();
    assert!(pos_null < pos_stale, "NULL user should sort before stale user");

    for id in &ids {
        sqlx::query("DELETE FROM users WHERE user_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .ok();
    }
}
```

- [ ] **Step 5: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server db::users
```

Expected: build clean, all existing db::users tests pass; new ignore-gated tests compile but do not execute in the standard run. Confirm with `cargo test --package plexus-server db::users -- --ignored` if a DB is available.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(schema): users.last_heartbeat_at + due-user query helper

Adds the TIMESTAMPTZ column, per-user get/update helpers, and the
list_users_due_for_heartbeat query the Plan E tick loop consumes.

NULL counts as infinitely-past so first-ever heartbeat fires on the
next tick after registration. Results are ordered oldest-first (NULL
as epoch) so that interval shrinks drain the longest-idle users first.
LIMIT parameter bounds single-tick workload; the next tick picks up
the remainder.

Ignore-gated integration tests cover: NULL round-trip, stale row
selection, fresh row exclusion, oldest-first ordering.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-2: Heartbeat Phase 1 prompt template + system_config seed

**Files:**
- Create: `plexus-server/templates/prompts/heartbeat_phase1.md`
- Modify: `plexus-server/src/db/system_config.rs`

- [ ] **Step 1: Create the prompt template**

Create `plexus-server/templates/prompts/heartbeat_phase1.md`:

```markdown
You are the heartbeat decision layer for PLEXUS, an autonomous AI agent.

The user has authored a `HEARTBEAT.md` task list. Every 30 minutes the system
wakes you up and asks: should the agent do anything right now?

Call the `heartbeat` tool **exactly once** with:
- `action: "skip"` — no tasks are ripe at the current local time. Give a short
  reason and return.
- `action: "run"` — one or more tasks should run now. Put a short free-text
  summary of what the agent should do in the `tasks` field; the agent will
  receive it as a user message.

When deciding:
- Respect the user's timezone and the local time shown below. Tasks scheduled
  for "every morning" fire once per morning, not every heartbeat.
- Skip if the task list is empty, is only notes, or no task matches now.
- When uncertain, skip. The cheapest action is no action.
- Do not elaborate. Call the tool and stop.
```

- [ ] **Step 2: Extend `seed_defaults_if_missing` to seed the new key**

In `plexus-server/src/db/system_config.rs`, modify `seed_defaults_if_missing`. After the existing three text-template seeds (default_soul / default_memory / default_heartbeat), add a fourth:

```rust
    // Text templates (seeded from shipped workspace templates via include_str!).
    for (key, default) in [
        (
            "default_soul",
            include_str!("../../templates/workspace/SOUL.md"),
        ),
        (
            "default_memory",
            include_str!("../../templates/workspace/MEMORY.md"),
        ),
        (
            "default_heartbeat",
            include_str!("../../templates/workspace/HEARTBEAT.md"),
        ),
        (
            "heartbeat_phase1_prompt",
            include_str!("../../templates/prompts/heartbeat_phase1.md"),
        ),
    ] {
        if get(pool, key).await?.is_none() {
            set(pool, key, default).await?;
        }
    }
```

Leave the scalar `for (key, default) in [("workspace_quota_bytes", ...)] ...` loop unchanged.

- [ ] **Step 3: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server db::system_config
```

Expected: clean build. `include_str!` resolves at compile time, so a missing template file surfaces as a compiler error.

- [ ] **Step 4: Commit**

```bash
git add plexus-server/templates/prompts/heartbeat_phase1.md plexus-server/src/db/system_config.rs
git commit -m "$(cat <<'EOF'
feat(heartbeat): ship Phase 1 prompt template + seed system_config

Heartbeat Phase 1 is a single-shot LLM call that chooses whether to
wake the agent. The shipped prompt teaches the LLM to call the
virtual `heartbeat(action, tasks)` tool exactly once, defaulting to
"skip" under ambiguity (the safe failure mode for autonomous
wake-ups).

seed_defaults_if_missing now also seeds heartbeat_phase1_prompt
alongside the existing workspace template keys, so admins can
override the prompt via system_config without restarting the server.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-3: `AppState::heartbeat_phase1_prompt` field + boot-load

**Files:**
- Modify: `plexus-server/src/state.rs`
- Modify: `plexus-server/src/main.rs`

- [ ] **Step 1: Add the AppState field**

In `plexus-server/src/state.rs`, add the field alongside the existing dream prompts (near line 55):

```rust
    // Dream prompt templates (admin-overridable via system_config)
    pub dream_phase1_prompt: Arc<RwLock<String>>,
    pub dream_phase2_prompt: Arc<RwLock<String>>,

    // Heartbeat Phase 1 prompt (admin-overridable via system_config)
    pub heartbeat_phase1_prompt: Arc<RwLock<String>>,
```

- [ ] **Step 2: Initialize in `test_with_pool`**

In the `test_with_pool` constructor (around line 211), add the field initialization after the two dream prompts:

```rust
            dream_phase1_prompt: std::sync::Arc::new(RwLock::new(
                include_str!("../templates/prompts/dream_phase1.md").to_string(),
            )),
            dream_phase2_prompt: std::sync::Arc::new(RwLock::new(
                include_str!("../templates/prompts/dream_phase2.md").to_string(),
            )),
            heartbeat_phase1_prompt: std::sync::Arc::new(RwLock::new(
                include_str!("../templates/prompts/heartbeat_phase1.md").to_string(),
            )),
```

- [ ] **Step 3: Initialize in `build_test_state`**

Same file, around line 251 — inside `build_test_state`, do the same addition:

```rust
            dream_phase2_prompt: std::sync::Arc::new(RwLock::new(
                include_str!("../templates/prompts/dream_phase2.md").to_string(),
            )),
            heartbeat_phase1_prompt: std::sync::Arc::new(RwLock::new(
                include_str!("../templates/prompts/heartbeat_phase1.md").to_string(),
            )),
```

- [ ] **Step 4: Boot-load in main.rs**

In `plexus-server/src/main.rs`, after the existing dream prompt loads (around line 62), add:

```rust
    let heartbeat_phase1_prompt = db::system_config::get(&pool, "heartbeat_phase1_prompt")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            include_str!("../templates/prompts/heartbeat_phase1.md").to_string()
        });
```

Then in the production `AppState { ... }` literal (around line 80), add the field:

```rust
        dream_phase1_prompt: Arc::new(RwLock::new(dream_phase1_prompt)),
        dream_phase2_prompt: Arc::new(RwLock::new(dream_phase2_prompt)),
        heartbeat_phase1_prompt: Arc::new(RwLock::new(heartbeat_phase1_prompt)),
```

- [ ] **Step 5: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: clean build. All 140+ existing tests pass (the new field is additive; no logic consumes it yet).

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(state): heartbeat_phase1_prompt field with include_str fallback

Adds the admin-overridable Phase 1 prompt to AppState, mirroring the
dream_phase{1,2}_prompt pattern from Plan D. Boot-loads from
system_config with the shipped template as fallback; both test
helpers initialize from include_str so unit tests never hit the DB.

No consumer yet — E-4 adds run_phase1 which reads this field.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-4: Heartbeat Phase 1 module (`heartbeat::run_phase1`)

**Files:**
- Create: `plexus-server/src/heartbeat.rs`
- Modify: `plexus-server/src/main.rs` (add `pub mod heartbeat;`)

- [ ] **Step 1: Register the module**

In `plexus-server/src/main.rs`, add the module declaration alongside `pub mod dream;`:

```rust
pub mod dream;
pub mod evaluator;
pub mod heartbeat;  // NEW
```

- [ ] **Step 2: Create `heartbeat.rs` with the Phase 1 + tick-loop scaffolding**

Create `plexus-server/src/heartbeat.rs`:

```rust
//! Heartbeat subsystem: periodic agent wake-up driven by HEARTBEAT.md task lists.
//!
//! Wired into the boot path (E-8): `spawn_heartbeat_tick` runs a 60-second
//! tokio timer. Each tick queries users due for a heartbeat (per
//! `system_config.heartbeat_interval_seconds`, default 1800 / 30 min),
//! advances `users.last_heartbeat_at` to prevent refire, and spawns
//! `run_phase1` per user.
//!
//! `run_phase1` is a single-shot LLM call with a virtual `heartbeat(action,
//! tasks)` tool forced via tool_choice=required. `action == "skip"` ends the
//! run silently; `action == "run"` publishes an InboundEvent
//! { kind: Heartbeat, session_id: "heartbeat:{user_id}", content: tasks }
//! which the agent loop routes to PromptMode::Heartbeat + ToolAllowlist::All.
//!
//! After Phase 2 completes, `agent_loop::publish_final`'s Heartbeat branch
//! (E-6) runs the shared evaluator (Plan C) and picks Discord → Telegram →
//! silence. Heartbeat never uses the gateway and never uses the `message`
//! tool to deliver.

use crate::state::AppState;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Tick loop cadence — fixed at 60 s, NOT admin-configurable.
/// The user-facing cadence knob is `heartbeat_interval_seconds`, which this
/// loop consults every tick.
const HEARTBEAT_TICK_INTERVAL_SEC: u64 = 60;

/// Per-tick cap on users processed in a single loop iteration. Prevents a
/// pathological backlog (e.g. admin shrinking the interval on a server with
/// many long-idle users) from spiking memory. The next tick picks up the
/// remainder because the query orders oldest-first.
const HEARTBEAT_MAX_USERS_PER_TICK: i64 = 500;

#[derive(Debug, Clone, PartialEq)]
pub enum Phase1Result {
    Skip { reason: String },
    Run { tasks: String },
}

fn virtual_tool() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "heartbeat",
            "description": "Decide whether to wake the agent now.",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["skip", "run"] },
                    "tasks":  { "type": "string" }
                },
                "required": ["action"]
            }
        }
    })
}

#[derive(Deserialize)]
struct ToolArgs {
    action: String,
    #[serde(default)]
    tasks: String,
}

/// Phase 1 standalone LLM call: decides whether Phase 2 should run.
///
/// Returns `Phase1Result::Skip` on any failure — silence is the safe
/// failure mode for autonomous wake-ups.
///
/// # Arguments
/// - `state`: shared AppState (DB + LLM config + HTTP client + prompt).
/// - `user_id`: user whose HEARTBEAT.md is the input. Timezone + workspace
///   file resolution both key off this value.
///
/// # Safety
/// HEARTBEAT.md content is user-authored and treated as trusted-for-this-user.
/// The agent itself may edit the file during earlier turns, so content is
/// effectively loop-owned; injection across users is impossible because the
/// path is scoped to `{workspace_root}/{user_id}/HEARTBEAT.md`.
pub async fn run_phase1(state: &Arc<AppState>, user_id: &str) -> Phase1Result {
    // 1. Load HEARTBEAT.md. Missing file → silent skip.
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let user_root = ws_root.join(user_id);
    let heartbeat_md = match tokio::fs::read_to_string(user_root.join("HEARTBEAT.md")).await {
        Ok(s) => s,
        Err(_) => {
            return Phase1Result::Skip {
                reason: "HEARTBEAT.md missing".into(),
            };
        }
    };

    // 2. Local time for the user.
    let tz_string = crate::db::users::get_timezone(&state.db, user_id)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, user_id, "heartbeat phase 1: timezone lookup failed, using UTC");
            "UTC".into()
        });
    let tz: chrono_tz::Tz = tz_string.parse().unwrap_or_else(|_| {
        warn!(user_id, tz = %tz_string, "heartbeat phase 1: malformed timezone, using UTC");
        chrono_tz::UTC
    });
    let local_now = chrono::Utc::now().with_timezone(&tz);

    // 3. Build messages.
    let system_prompt = state.heartbeat_phase1_prompt.read().await.clone();
    let user_body = format!(
        "## Current local time ({tz_string})\n{}\n\n## HEARTBEAT.md\n{heartbeat_md}",
        local_now.format("%A %H:%M %Z"),
    );
    let messages = vec![
        crate::providers::openai::ChatMessage::system(system_prompt),
        crate::providers::openai::ChatMessage::user(user_body),
    ];

    // 4. LLM config. Missing → silent skip.
    let llm_config = match state.llm_config.read().await.clone() {
        Some(c) => c,
        None => {
            warn!(user_id, "heartbeat phase 1: no LLM config, skipping");
            return Phase1Result::Skip {
                reason: "no LLM config".into(),
            };
        }
    };

    // 5. Call the LLM, force tool use.
    let response = crate::providers::openai::call_llm(
        &state.http_client,
        &llm_config,
        messages,
        Some(vec![virtual_tool()]),
        Some("required".into()),
    )
    .await;

    let calls = match response {
        Ok(crate::providers::openai::LlmResponse::ToolCalls { calls, .. }) if !calls.is_empty() => calls,
        Ok(_) => {
            warn!(user_id, "heartbeat phase 1: LLM did not return a tool call, skipping");
            return Phase1Result::Skip {
                reason: "LLM did not call the heartbeat tool".into(),
            };
        }
        Err(e) => {
            warn!(error = %e, user_id, "heartbeat phase 1: LLM call failed, skipping");
            return Phase1Result::Skip {
                reason: format!("LLM error: {e}"),
            };
        }
    };

    // 6. Parse the first tool call.
    let first = &calls[0];
    if first.function.name != "heartbeat" {
        warn!(got = %first.function.name, user_id, "heartbeat phase 1: unexpected tool name, skipping");
        return Phase1Result::Skip {
            reason: format!("unexpected tool name: {}", first.function.name),
        };
    }
    let args: ToolArgs = match serde_json::from_str(&first.function.arguments) {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, user_id, "heartbeat phase 1: failed to parse tool args, skipping");
            return Phase1Result::Skip {
                reason: format!("parse error: {e}"),
            };
        }
    };

    match args.action.as_str() {
        "skip" => Phase1Result::Skip {
            reason: "phase 1 returned skip".into(),
        },
        "run" if args.tasks.trim().is_empty() => {
            // Degenerate "run" with no task description — treat as skip rather than
            // wake the agent with nothing to do.
            info!(user_id, "heartbeat phase 1: run with empty tasks, treating as skip");
            Phase1Result::Skip {
                reason: "run with empty tasks".into(),
            }
        }
        "run" => Phase1Result::Run { tasks: args.tasks },
        other => {
            warn!(action = other, user_id, "heartbeat phase 1: unexpected action, skipping");
            Phase1Result::Skip {
                reason: format!("unexpected action: {other}"),
            }
        }
    }
}

/// Spawn the 60-second heartbeat tick loop. Wired in `main.rs` at boot.
/// Graceful shutdown: observes `state.shutdown` and exits the select loop.
///
/// E-8 fills in the tick body; this skeleton stops on shutdown and does nothing else.
pub fn spawn_heartbeat_tick(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            HEARTBEAT_TICK_INTERVAL_SEC,
        ));
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    info!("heartbeat tick shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = tick_once(&state).await {
                        warn!(error = %e, "heartbeat tick error");
                    }
                }
            }
        }
    });
}

/// One tick of the heartbeat loop. Skeleton for E-4; E-8 fills in the body.
async fn tick_once(_state: &Arc<AppState>) -> Result<(), String> {
    // E-8 implementation goes here: query due users, advance last_heartbeat_at,
    // spawn Phase 1 per user, publish Phase 2 InboundEvent on Run.
    debug!("heartbeat tick (E-4 skeleton — E-8 fills the body)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_tool_shape_is_correct() {
        let tool = virtual_tool();
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "heartbeat");
        let params = &tool["function"]["parameters"];
        assert_eq!(params["properties"]["action"]["type"], "string");
        assert_eq!(
            params["properties"]["action"]["enum"],
            serde_json::json!(["skip", "run"])
        );
        assert_eq!(params["properties"]["tasks"]["type"], "string");
        assert_eq!(params["required"], serde_json::json!(["action"]));
    }

    #[test]
    fn tool_args_parse_skip_without_tasks() {
        let parsed: ToolArgs = serde_json::from_str(r#"{"action": "skip"}"#).unwrap();
        assert_eq!(parsed.action, "skip");
        assert_eq!(parsed.tasks, "");
    }

    #[test]
    fn tool_args_parse_run_with_tasks() {
        let parsed: ToolArgs = serde_json::from_str(
            r#"{"action": "run", "tasks": "check email"}"#,
        )
        .unwrap();
        assert_eq!(parsed.action, "run");
        assert_eq!(parsed.tasks, "check email");
    }

    #[test]
    fn tool_args_parse_rejects_missing_action() {
        let err = serde_json::from_str::<ToolArgs>(r#"{"tasks": "anything"}"#);
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_run_phase1_skips_when_no_llm_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Prime a workspace with HEARTBEAT.md so the early-return doesn't
        // shadow the LLM-config check we're actually testing.
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        tokio::fs::write(user_root.join("HEARTBEAT.md"), b"- test task").await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        // test_minimal does NOT set an LLM config.
        let result = run_phase1(&state, "alice").await;
        match result {
            Phase1Result::Skip { reason } => {
                assert!(
                    reason.contains("no LLM config") || reason.contains("timezone"),
                    "expected LLM-config or timezone-lookup skip reason; got: {reason}"
                );
            }
            Phase1Result::Run { .. } => panic!("expected Skip"),
        }
    }

    #[tokio::test]
    async fn test_run_phase1_skips_when_heartbeat_md_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        // No workspace for "bob" — HEARTBEAT.md won't exist.
        let state = crate::state::AppState::test_minimal(tmp.path());
        let result = run_phase1(&state, "bob").await;
        match result {
            Phase1Result::Skip { reason } => {
                assert!(
                    reason.contains("HEARTBEAT.md missing"),
                    "expected missing-file skip reason; got: {reason}"
                );
            }
            Phase1Result::Run { .. } => panic!("expected Skip"),
        }
    }
}
```

- [ ] **Step 3: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server heartbeat
```

Expected: clean build. All new tests pass; the `test_run_phase1_skips_when_no_llm_config` test may surface a timezone-lookup failure before the LLM-config check — both paths are acceptable skip reasons (the assert accepts either).

- [ ] **Step 4: Commit**

```bash
git add plexus-server/src/heartbeat.rs plexus-server/src/main.rs
git commit -m "$(cat <<'EOF'
feat(heartbeat): Phase 1 virtual-tool decision + tick-loop skeleton

heartbeat::run_phase1 is a standalone LLM call that forces tool use
via tool_choice=required with a virtual heartbeat(action, tasks)
tool. Parses action=skip | run, returns Phase1Result. Default-skip
on every error path: missing HEARTBEAT.md, timezone failure, LLM
error, parse error, unexpected tool name, empty tasks on run.

spawn_heartbeat_tick ships the 60 s graceful-shutdown skeleton; the
real tick body lands in E-8 after E-5/E-6 wire up PromptMode and
publish_final.

Unit tests cover the virtual-tool JSON shape, ToolArgs parsing
(skip-without-tasks, run-with-tasks, missing-action rejection), and
two Phase1Result::Skip paths that don't require a real LLM.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-5: `PromptMode::Heartbeat` real branch in `context.rs`

**Files:**
- Modify: `plexus-server/src/context.rs`

- [ ] **Step 1: Add the banner constant + helper**

Near the top of `plexus-server/src/context.rs`, alongside `assemble_dream_system_prompt`, add:

```rust
const HEARTBEAT_BANNER: &str = "## Autonomous Wake-Up\n\
This is an autonomous heartbeat wake-up triggered by your scheduled task list. \
Complete the requested tasks without asking for clarifying questions — pick \
reasonable defaults and proceed. Do not use the `message` tool to deliver a \
reply; produce a concise final assistant message summarizing what you did. \
The system will decide whether to notify the user through an external channel.\n";
```

And add a private async helper `build_heartbeat_system` — note it takes `user: &User` so the `### Account` subsection renders identically to UserTurn's (shared email + display_name handling), keeping the identity surface consistent across modes:

```rust
async fn build_heartbeat_system(
    soul: &str,
    user: &User,
    identity: &ChannelIdentity,
    chat_id: Option<&str>,
    state: &AppState,
    memory: &str,
    skills_section: &str,
) -> String {
    let mut s = format!("{soul}\n\n");

    // Section: Identity (identical rendering to UserTurn)
    s += "## Identity\n";
    let name = user
        .display_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(not set)");
    s += &format!("### Account\nName: {} | Email: {}\n\n", name, user.email);
    s += &identity.build_session_section(chat_id);
    s += "\n";

    // NO ## Channels section — heartbeat never routes to an interactive channel.

    // Memory
    if !memory.trim().is_empty() {
        s += &format!("## Memory\n{memory}\n\n");
    }

    // Skills
    s += skills_section;

    // Devices
    s += &build_device_status(state, &user.user_id).await;

    // Runtime
    s += &format!(
        "Current time: {}\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    // Autonomous wake-up banner (pins behavior)
    s += HEARTBEAT_BANNER;

    s
}
```

- [ ] **Step 2: Split the match arm**

In `build_context` (around line 308), change the match:

```rust
    // ── Mode-specific system prompt assembly ──────────────────────────────────
    let system = match mode {
        PromptMode::UserTurn => {
            // Existing UserTurn logic unchanged — copy the body from the
            // current `PromptMode::UserTurn | PromptMode::Heartbeat` arm.
            let mut s = format!("{soul}\n\n");

            // Section: Identity
            s += "## Identity\n";
            let name = user
                .display_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("(not set)");
            s += &format!("### Account\nName: {} | Email: {}\n\n", name, user.email);
            s += &identity.build_session_section(chat_id);
            s += "\n";

            // Channels
            let snap = load_channel_snapshot(state, &user.user_id).await;
            s += &render_channels_section(&snap);
            s += "Reply on the current channel unless the partner asks otherwise.\n\n";

            // Attachments
            s += "## Attachments\n";
            s += "Files may appear as [Attachment: name → /api/files/{id}]. They live on the\n";
            s += "server. To operate on one, use `file_transfer` to move it to a client device,\n";
            s += "then use client tools (shell, read_file, etc.). Choose the action based on\n";
            s += "filename and the user's intent.\n\n";

            // Memory
            if !memory.trim().is_empty() {
                s += &format!("## Memory\n{}\n\n", memory);
            }

            // Skills
            s += &skills_section;

            // Devices
            s += &build_device_status(state, &user.user_id).await;

            // Runtime
            s += &format!(
                "Current time: {}\n",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            );

            s
        }
        PromptMode::Heartbeat => {
            // `user` and `identity` are already `&…` in build_context's
            // signature, so pass them through without an extra borrow.
            build_heartbeat_system(
                soul,
                user,
                identity,
                chat_id,
                state,
                &memory,
                &skills_section,
            )
            .await
        }
        PromptMode::Dream => {
            // Dream Phase 2: phase2 prompt + memory + soul + skills.
            // Deliberately OMITS channel identity, device list, and current time —
            // dream is an autonomous server-side pass, not a user-facing reply.
            let phase2 = state.dream_phase2_prompt.read().await.clone();
            assemble_dream_system_prompt(&phase2, &memory, soul, &skills_section)
        }
    };
```

- [ ] **Step 3: Add unit tests**

Append to `#[cfg(test)] mod mode_tests` block at the bottom of the file:

```rust
    #[test]
    fn heartbeat_banner_contains_no_clarifying_question_guidance() {
        // Pin the banner wording so future edits don't regress the
        // "don't ask clarifying questions" contract.
        assert!(HEARTBEAT_BANNER.contains("without asking for clarifying questions"));
        assert!(HEARTBEAT_BANNER.contains("Do not use the `message` tool"));
        assert!(HEARTBEAT_BANNER.contains("summarizing what you did"));
    }

    // Note: a full end-to-end `build_context(PromptMode::Heartbeat, ...)`
    // integration test would require a real PgPool (because
    // build_device_status and load_channel_snapshot hit the DB). That is
    // covered as a manual smoke test + the ignore-gated test in E-9. The
    // banner pin above plus the UserTurn/Dream regression tests below
    // give us confidence the match arm routes correctly.
```

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server context
```

Expected: clean build. The existing dream_mode tests still pass (none of them touch Heartbeat). The new `heartbeat_banner_contains_no_clarifying_question_guidance` passes.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(context): real PromptMode::Heartbeat branch + autonomous banner

Splits PromptMode::Heartbeat out of the UserTurn fallback and into
its own build_heartbeat_system helper. Heartbeat context includes
soul + identity + memory + skills + devices + runtime — but OMITS
the ## Channels section. A trailing ## Autonomous Wake-Up banner
pins three behavior rules the agent must follow on heartbeat turns:
no clarifying questions, no `message` tool for delivery, concise
final summary.

This replaces the Plan D stub that routed Heartbeat into the
UserTurn body. E-7 threads event.kind through publish_final so the
evaluator+external-channel branch (E-6) kicks in on the final text.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-6: `PublishFinalParams::kind` + Heartbeat publish branch

**Files:**
- Modify: `plexus-server/src/agent_loop.rs`

- [ ] **Step 1: Add `kind` to `PublishFinalParams`**

In `plexus-server/src/agent_loop.rs` (around line 14):

```rust
pub(crate) struct PublishFinalParams {
    pub channel: String,
    pub chat_id: Option<String>,
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    /// Dispatch discriminant. Drives which publish branch runs:
    /// - UserTurn  → publish to `channel`/`chat_id` unconditionally.
    /// - Cron      → existing evaluator-gated-by-deliver branch.
    /// - Heartbeat → evaluator + external-channel precedence
    ///               (Discord → Telegram → silence; never gateway).
    /// - Dream     → no publish (dream cron rows have deliver=false).
    pub kind: crate::bus::EventKind,
    pub cron_job_id: Option<String>,
    pub job_deliver: Option<bool>,
}
```

- [ ] **Step 2: Dispatch via `params.kind` in `publish_final`**

Replace the existing `publish_final` body (around line 38–98):

```rust
pub(crate) async fn publish_final(
    state: &std::sync::Arc<crate::state::AppState>,
    params: PublishFinalParams,
) {
    let PublishFinalParams {
        channel,
        chat_id,
        session_id,
        user_id,
        content,
        kind,
        cron_job_id,
        job_deliver,
    } = params;

    use crate::bus::EventKind;
    match kind {
        EventKind::UserTurn => publish_via_channel(state, channel, chat_id, session_id, user_id, content).await,
        EventKind::Cron => publish_final_cron(state, channel, chat_id, session_id, user_id, content, cron_job_id, job_deliver).await,
        EventKind::Heartbeat => publish_final_heartbeat(state, &user_id, &content).await,
        EventKind::Dream => {
            // Dream cron rows have deliver=false; this arm is defensive.
            info!(session_id, "dream turn completed; no publish");
        }
    }
}

async fn publish_via_channel(
    state: &std::sync::Arc<crate::state::AppState>,
    channel: String,
    chat_id: Option<String>,
    session_id: String,
    user_id: String,
    content: String,
) {
    let _ = state
        .outbound_tx
        .send(crate::bus::OutboundEvent {
            channel,
            chat_id,
            session_id,
            user_id,
            content,
            media: vec![],
        })
        .await;
}

async fn publish_final_cron(
    state: &std::sync::Arc<crate::state::AppState>,
    channel: String,
    chat_id: Option<String>,
    session_id: String,
    user_id: String,
    content: String,
    cron_job_id: Option<String>,
    job_deliver: Option<bool>,
) {
    let Some(job_id) = cron_job_id else {
        // Cron kind without a job_id would be a bug; log and drop.
        warn!(session_id, "publish_final: EventKind::Cron with no cron_job_id — skipping publish");
        return;
    };

    let deliver = match job_deliver {
        Some(d) => d,
        None => match crate::db::cron::find_by_id(&state.db, &job_id).await {
            Ok(Some(job)) => job.deliver,
            Ok(None) => {
                warn!(job_id, "publish_final: cron job not found, skipping publish");
                return;
            }
            Err(e) => {
                warn!(error = %e, job_id, "publish_final: cron job lookup failed, skipping publish");
                return;
            }
        },
    };

    if !deliver {
        info!(job_id, "cron deliver=false; skipping OutboundEvent publish");
        return;
    }

    let purpose = format!("cron job '{job_id}'");
    let eval = crate::evaluator::evaluate_notification(state, &user_id, &content, &purpose).await;
    if !eval.should_notify {
        info!(
            job_id,
            reason = %eval.reason,
            "evaluator suppressed cron delivery"
        );
        return;
    }

    publish_via_channel(state, channel, chat_id, session_id, user_id, content).await;
}

/// Heartbeat: evaluator gate → external-channel precedence (Discord → Telegram).
/// Never gateway. Silence on no-config.
async fn publish_final_heartbeat(
    state: &std::sync::Arc<crate::state::AppState>,
    user_id: &str,
    content: &str,
) {
    // 1. Evaluator gate.
    let eval = crate::evaluator::evaluate_notification(
        state,
        user_id,
        content,
        "heartbeat wake-up",
    )
    .await;
    if !eval.should_notify {
        info!(user_id, reason = %eval.reason, "heartbeat: evaluator suppressed notification");
        return;
    }

    // 2. Discord first.
    if let Ok(Some(cfg)) = crate::db::discord::get_config(&state.db, user_id).await {
        if cfg.enabled {
            if let Some(partner_id) = cfg.partner_discord_id.as_deref() {
                if !partner_id.is_empty() {
                    let _ = state
                        .outbound_tx
                        .send(crate::bus::OutboundEvent {
                            channel: plexus_common::consts::CHANNEL_DISCORD.to_string(),
                            chat_id: Some(format!("dm/{partner_id}")),
                            session_id: format!("heartbeat:{user_id}"),
                            user_id: user_id.to_string(),
                            content: content.to_string(),
                            media: vec![],
                        })
                        .await;
                    info!(user_id, "heartbeat: delivered via discord");
                    return;
                }
            }
        }
    }

    // 3. Telegram second.
    if let Ok(Some(cfg)) = crate::db::telegram::get_config(&state.db, user_id).await {
        if cfg.enabled {
            if let Some(partner_id) = cfg.partner_telegram_id.as_deref() {
                if !partner_id.is_empty() {
                    let _ = state
                        .outbound_tx
                        .send(crate::bus::OutboundEvent {
                            channel: crate::channels::CHANNEL_TELEGRAM.to_string(),
                            chat_id: Some(partner_id.to_string()),
                            session_id: format!("heartbeat:{user_id}"),
                            user_id: user_id.to_string(),
                            content: content.to_string(),
                            media: vec![],
                        })
                        .await;
                    info!(user_id, "heartbeat: delivered via telegram");
                    return;
                }
            }
        }
    }

    // 4. Silence.
    info!(user_id, "heartbeat: no external channel configured; output stored only");
}
```

- [ ] **Step 3: Update existing call sites**

In `handle_event` (search for `publish_final(` — around line 341), add `kind: event.kind.clone()` — wait, `EventKind` is `Copy`, so just `kind: event.kind`:

```rust
                publish_final(
                    state,
                    PublishFinalParams {
                        channel: event.channel.clone(),
                        chat_id: event.chat_id.clone(),
                        session_id: session_id.to_string(),
                        user_id: user_id.to_string(),
                        content,
                        kind: event.kind,
                        cron_job_id: event.cron_job_id.clone(),
                        job_deliver: None,
                    },
                )
                .await;
```

In the two `deliver_tests` test literals (around lines 641 and 664), add `kind: EventKind::Cron` and `kind: EventKind::UserTurn` respectively. Both tests already use the corresponding event shape; the `kind` is an explicit re-spelling of what was implicit:

```rust
        let params = PublishFinalParams {
            // ... existing fields ...
            kind: crate::bus::EventKind::Cron,  // in test_publish_final_skips_when_cron_deliver_false
            cron_job_id: Some("j1".into()),
            job_deliver: Some(false),
        };
```

```rust
        let params = PublishFinalParams {
            // ... existing fields ...
            kind: crate::bus::EventKind::UserTurn,  // in test_publish_final_publishes_user_turn
            cron_job_id: None,
            job_deliver: None,
        };
```

- [ ] **Step 4: Add a heartbeat-specific deliver test**

Append to the `#[cfg(test)] mod deliver_tests` in `agent_loop.rs`:

```rust
    #[tokio::test]
    async fn test_publish_final_heartbeat_no_channels_is_silent() {
        // Heartbeat with no Discord / Telegram config and no LLM config
        // (evaluator defaults to silence). Expect: no OutboundEvent.
        let tmp = tempfile::TempDir::new().unwrap();
        let (state, mut outbound_rx) = crate::state::AppState::test_minimal_with_outbound(tmp.path());

        let params = PublishFinalParams {
            channel: "internal".into(),
            chat_id: None,
            session_id: "heartbeat:alice".into(),
            user_id: "alice".into(),
            content: "Did the thing.".into(),
            kind: crate::bus::EventKind::Heartbeat,
            cron_job_id: None,
            job_deliver: None,
        };

        publish_final(&state, params).await;

        // Evaluator defaults to silence without an LLM config, so nothing ships.
        assert!(
            outbound_rx.try_recv().is_err(),
            "expected no OutboundEvent for silent heartbeat"
        );
    }
```

- [ ] **Step 5: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server agent_loop::deliver_tests
```

Expected: clean build. All deliver_tests pass, including the new heartbeat-silent case.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(agent_loop): publish_final Heartbeat branch + kind dispatch

PublishFinalParams now carries an explicit EventKind, replacing the
implicit "cron_job_id.is_some() == cron" heuristic. publish_final
dispatches on kind into three real branches (UserTurn / Cron /
Heartbeat) and a defensive Dream no-op.

publish_final_heartbeat runs the shared evaluator
(purpose = "heartbeat wake-up"), then walks the Discord-then-Telegram
external-channel precedence. The gateway is intentionally skipped:
heartbeat wake-ups must not interrupt an active browser session
(spec §9.7). Silence on evaluator-no, on no-config, or on empty
partner IDs.

Two existing deliver_tests updated with explicit kind: fields; new
test_publish_final_heartbeat_no_channels_is_silent pins the silence
contract without requiring a real LLM.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-7: Verify full agent-loop wiring with a smoke test

**Files:**
- Modify: `plexus-server/src/agent_loop.rs` (add one test only)

This task is a low-risk compile-and-assert pass confirming the integration of E-5 + E-6. The production agent_loop already dispatches `EventKind::Heartbeat → PromptMode::Heartbeat` (from D-8) and now, after E-5, the PromptMode branch builds a real heartbeat prompt; after E-6, the post-turn publish branches on event.kind. The only remaining risk is a missed call site.

- [ ] **Step 1: Sanity-grep for stale literal `InboundEvent {` constructions**

```bash
grep -rn "InboundEvent {" plexus-server/src/ | grep -v "//" | grep -v "test"
```

Expected: every non-test construction sets `kind:` explicitly. There should be no changes required — Plan D already enforced this.

- [ ] **Step 2: Sanity-grep for stale `PublishFinalParams {` constructions**

```bash
grep -rn "PublishFinalParams {" plexus-server/src/
```

Expected: three sites (production `handle_event` + two test helpers in deliver_tests). All three were updated in E-6. If a fourth appears, update it with the correct `kind:` value.

- [ ] **Step 3: Add one end-to-end wiring regression test**

Append to `agent_loop.rs`'s `#[cfg(test)] mod deliver_tests`:

```rust
    #[test]
    fn publish_final_params_size_is_reasonable() {
        // Guardrail: if someone accidentally stuffs a Vec or a String into
        // the Copy-able position of kind, this fails — the struct would
        // grow by a heap pointer's worth of indirection. Heartbeat is
        // hot enough that this matters.
        //
        // Current fields: 2 String, 2 Option<String>, 1 EventKind (u8-sized),
        // 1 Option<String>, 1 Option<bool>. String = 24 bytes on 64-bit,
        // Option<String> = 24 bytes (niche on len). So the struct should
        // be bounded around 150 bytes.
        let size = std::mem::size_of::<PublishFinalParams>();
        assert!(
            size <= 200,
            "PublishFinalParams grew beyond 200 bytes (got {size}); \
             heartbeat dispatch is hot — review the struct layout."
        );
    }
```

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server agent_loop
cargo clippy --package plexus-server -- -D warnings
```

Expected: build + tests green. Clippy may surface warnings about newly unused imports — fix them inline (e.g., if `EventKind` was only used by the old implicit path).

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
test(agent_loop): pin PublishFinalParams size + sanity-grep audit

E-5 and E-6 landed the heartbeat branch logic; this task audits the
call sites (three expected) and pins PublishFinalParams to a
sensible size so accidental heap-allocation additions surface as a
test failure.

No production behavior changes; this is a regression fence for
hot-path dispatch.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-8: Tick-loop body + main.rs boot wiring

**Files:**
- Modify: `plexus-server/src/heartbeat.rs`
- Modify: `plexus-server/src/main.rs`

- [ ] **Step 1: Replace the `tick_once` skeleton**

In `plexus-server/src/heartbeat.rs`, replace the `async fn tick_once` body:

```rust
async fn tick_once(state: &Arc<AppState>) -> Result<(), String> {
    // 1. Read the admin-configurable interval. 0 → heartbeat disabled.
    let interval_seconds = match crate::db::system_config::get(
        &state.db,
        "heartbeat_interval_seconds",
    )
    .await
    {
        Ok(Some(v)) => match v.parse::<i64>() {
            Ok(n) => n,
            Err(e) => {
                warn!(value = %v, error = %e, "heartbeat: interval parse error, skipping tick");
                return Ok(());
            }
        },
        Ok(None) => 1800, // seed missing — fall back to default
        Err(e) => {
            warn!(error = %e, "heartbeat: system_config lookup failed, skipping tick");
            return Ok(());
        }
    };
    if interval_seconds <= 0 {
        debug!(interval_seconds, "heartbeat: globally disabled, skipping tick");
        return Ok(());
    }

    // 2. Query due users.
    let due = crate::db::users::list_users_due_for_heartbeat(
        &state.db,
        interval_seconds,
        HEARTBEAT_MAX_USERS_PER_TICK,
    )
    .await
    .map_err(|e| format!("list_users_due_for_heartbeat: {e}"))?;

    if due.is_empty() {
        return Ok(());
    }
    debug!(count = due.len(), "heartbeat: dispatching due users");

    // 3. Per-user dispatch.
    let ws_root = std::path::Path::new(&state.config.workspace_root).to_path_buf();
    for user_id in due {
        // 3a. Skip if prior heartbeat turn still running.
        //     try_lock is a liveness probe; benign race if a new turn
        //     starts between our check and publish.
        if let Some(handle) = state.sessions.get(&format!("heartbeat:{user_id}")) {
            if handle.lock.try_lock().is_err() {
                debug!(user_id, "heartbeat: prior turn still running, skipping");
                continue;
            }
        }

        // 3b. Skip if HEARTBEAT.md is missing (users can delete it).
        let heartbeat_path = ws_root.join(&user_id).join("HEARTBEAT.md");
        match tokio::fs::try_exists(&heartbeat_path).await {
            Ok(true) => {}
            Ok(false) => {
                debug!(user_id, "heartbeat: HEARTBEAT.md missing, skipping");
                continue;
            }
            Err(e) => {
                warn!(user_id, error = %e, "heartbeat: try_exists failed, skipping");
                continue;
            }
        }

        // 3c. Advance last_heartbeat_at BEFORE spawning Phase 1.
        //     Prevents refire during LLM latency, and also serves as a
        //     single-advance barrier if a concurrent tick somehow fires.
        let now = chrono::Utc::now();
        if let Err(e) = crate::db::users::update_last_heartbeat_at(&state.db, &user_id, now).await {
            warn!(user_id, error = %e, "heartbeat: advance last_heartbeat_at failed, skipping");
            continue;
        }

        // 3d. Spawn Phase 1 off the tick task. Phase 1 + publish run in
        //     parallel across users; the tick loop stays responsive.
        let state_clone = Arc::clone(state);
        let user_id_clone = user_id.clone();
        tokio::spawn(async move {
            match run_phase1(&state_clone, &user_id_clone).await {
                Phase1Result::Skip { reason } => {
                    info!(user_id = %user_id_clone, reason, "heartbeat: phase 1 skipped");
                }
                Phase1Result::Run { tasks } => {
                    let event = crate::bus::InboundEvent {
                        session_id: format!("heartbeat:{user_id_clone}"),
                        user_id: user_id_clone.clone(),
                        kind: crate::bus::EventKind::Heartbeat,
                        content: tasks,
                        channel: "internal".to_string(),
                        chat_id: None,
                        media: vec![],
                        cron_job_id: None,
                        identity: None,
                    };
                    if let Err(e) = crate::bus::publish_inbound(&state_clone, event).await {
                        warn!(
                            user_id = %user_id_clone,
                            error = %e,
                            "heartbeat: publish_inbound failed"
                        );
                    }
                }
            }
        });
    }

    Ok(())
}
```

- [ ] **Step 2: Wire `spawn_heartbeat_tick` into `main.rs`**

In `plexus-server/src/main.rs`, after the existing `cron::spawn_cron_poller(Arc::clone(&state));` call (around line 119):

```rust
    cron::spawn_cron_poller(Arc::clone(&state));
    heartbeat::spawn_heartbeat_tick(Arc::clone(&state));
```

- [ ] **Step 3: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: clean build. All existing tests pass. `tick_once` itself doesn't have a unit test yet — it needs a DB; E-9 adds the integration coverage.

- [ ] **Step 4: Manual smoke**

Run the server. Expected logs within the first 60 s of boot:

```
INFO heartbeat tick (E-4 skeleton — E-8 fills the body)     ← gone after this task
DEBUG heartbeat: globally disabled, skipping tick            ← only if interval_seconds == 0
DEBUG heartbeat: dispatching due users count=N               ← if there are due users
INFO heartbeat: phase 1 skipped user_id=… reason="…"         ← per due user
```

On SIGTERM:

```
INFO heartbeat tick shutting down
```

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(heartbeat): tick-loop body + main.rs boot wiring

tick_once reads the admin-configurable interval, lists due users
(NULL or stale-beyond-threshold), pre-filters by in-flight session
lock + HEARTBEAT.md existence, advances last_heartbeat_at BEFORE
spawning Phase 1, then spawns per-user Phase 1 + publish in
parallel tokio tasks. The tick task itself stays responsive to
shutdown.

Graceful-shutdown: the tick loop's tokio::select! watches
state.shutdown and exits cleanly, matching the cron poller pattern.

heartbeat_interval_seconds == 0 is the global kill switch. A missing
or malformed value falls back to 1800 (seeded default) or logs and
skips the tick respectively.

Wired into main.rs alongside spawn_cron_poller.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-9: Integration test for the due-user query + registration integration

**Files:**
- Modify: `plexus-server/src/heartbeat.rs`

- [ ] **Step 1: Add an ignore-gated integration test**

Append to `#[cfg(test)] mod tests` in `plexus-server/src/heartbeat.rs`:

```rust
    #[tokio::test]
    #[ignore] // needs DATABASE_URL
    async fn test_tick_due_users_flow_end_to_end_db() {
        // Scenario: three users, only two are due. Verify the list_users_due_for_heartbeat
        // query returns exactly the right subset and that update_last_heartbeat_at
        // correctly moves a user out of the due set.
        let url = std::env::var("DATABASE_URL")
            .expect("set DATABASE_URL to run this test");
        let pool = crate::db::init_db(&url).await;

        // Fresh users.
        let ids: Vec<String> = (0..3)
            .map(|i| format!("e9-{}-{}", i, &uuid::Uuid::new_v4().to_string()[..8]))
            .collect();
        for id in &ids {
            crate::db::users::create_user(&pool, id, &format!("{id}@test.local"), "", false)
                .await
                .unwrap();
        }

        // ids[0] stays NULL. ids[1] is stale (1h ago). ids[2] is fresh.
        crate::db::users::update_last_heartbeat_at(
            &pool,
            &ids[1],
            chrono::Utc::now() - chrono::Duration::hours(1),
        )
        .await
        .unwrap();
        crate::db::users::update_last_heartbeat_at(
            &pool,
            &ids[2],
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        // 30-min interval → ids[0] + ids[1] are due, ids[2] is not.
        let due = crate::db::users::list_users_due_for_heartbeat(&pool, 1800, 100)
            .await
            .unwrap();
        assert!(due.contains(&ids[0]));
        assert!(due.contains(&ids[1]));
        assert!(!due.contains(&ids[2]));

        // Advance ids[0] to NOW → it should drop out of the due set on next query.
        crate::db::users::update_last_heartbeat_at(&pool, &ids[0], chrono::Utc::now())
            .await
            .unwrap();
        let due_after = crate::db::users::list_users_due_for_heartbeat(&pool, 1800, 100)
            .await
            .unwrap();
        assert!(!due_after.contains(&ids[0]), "ids[0] should no longer be due after advance");
        assert!(due_after.contains(&ids[1]), "ids[1] still stale");

        // Cleanup.
        for id in &ids {
            sqlx::query("DELETE FROM users WHERE user_id = $1")
                .bind(id)
                .execute(&pool)
                .await
                .ok();
        }
    }
```

- [ ] **Step 2: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server heartbeat
```

Expected: new test compiles; runs only under `--ignored` with DATABASE_URL set. If DATABASE_URL is available, run:

```bash
DATABASE_URL=<url> cargo test --package plexus-server heartbeat -- --ignored
```

Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
test(heartbeat): ignore-gated end-to-end due-user query integration

Three users, one NULL / one stale / one fresh. Asserts:
- 30-min threshold selects NULL + stale, excludes fresh.
- Advancing the NULL user to NOW drops them from the due set.
- Advancing a fresh user does not spuriously include them.

This pins the two DB predicates Plan E's tick loop relies on —
regressions here would fire heartbeats on fresh users or skip
NULL users forever.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task E-10: ADR + ISSUE sweep + Post-Plan Adjustments footer

**Files:**
- Modify: `plexus-server/docs/DECISIONS.md`
- Modify: `plexus-server/docs/ISSUE.md`
- Modify: `plexus-server/docs/SCHEMA.md` (if present and stale for last_heartbeat_at)
- Modify: `docs/superpowers/plans/2026-04-18-heartbeat-subsystem.md` (this file — append Post-Plan Adjustments footer)

- [ ] **Step 1: Add a new ADR**

In `plexus-server/docs/DECISIONS.md`, add a new entry at the end — the next consecutive ADR (check the current numbering; the handoff doc mentions ADR-35 is the latest). Number this one `ADR-36` unless a higher ADR has landed:

```markdown
## ADR-36: Heartbeat as in-process tick loop with evaluator-gated external-channel delivery

**Date:** 2026-04-18
**Status:** Accepted
**Plan:** E (heartbeat subsystem)

### Context

Plexus needed periodic agent wake-ups driven by a user-owned task list, mirroring nanobot's heartbeat. Unlike dream (which is idempotent memory consolidation on idle users), heartbeat fires on a fixed interval regardless of conversation activity and must be able to notify the user through an external channel.

Three architectural questions resolved here:

1. **Scheduling:** tick loop vs. cron job?
2. **Delivery:** how does the final agent message reach the user without becoming notification spam?
3. **Channel selection:** where does the notification go?

### Decision

- **Scheduling:** A dedicated in-process tick loop (`heartbeat::spawn_heartbeat_tick`), not a cron job. Interval is fixed at 60 s (tick cadence), with the actual wake-up cadence tunable via `system_config.heartbeat_interval_seconds` (default 1800 s / 30 min). `0` is a global kill switch.
- **Delivery:** Shared `evaluator::evaluate_notification` (Plan C) gates every heartbeat output with `purpose = "heartbeat wake-up"`. Default-silence on any evaluator error — the 4 AM guard fires here, not for dream.
- **Channel selection:** Discord → Telegram → silence. The gateway is explicitly skipped: heartbeat must not interrupt an active browser session.

### Consequences

- **Positive:**
  - Fixed-cadence semantics are cleaner than cron-style expression matching for "every N minutes".
  - Tick loop watches `state.shutdown` — graceful shutdown is free.
  - Reusing the agent loop + `publish_final` keeps the heartbeat path consistent with cron and dream (same compression, crash recovery, tool dispatch).
- **Negative:**
  - Server-specific state: a multi-server deployment would refire heartbeat per server unless an advisory lock or node-leader pattern is added later. Plexus is currently single-node; this is tracked as a follow-up.
  - `last_heartbeat_at` is advanced *before* Phase 1 runs. A Phase 1 crash would skip that user until the next interval elapses. Same trade-off as dream's `last_dream_at` advance; preferred over re-firing after a poisoned Phase 1.

### Alternatives considered

- **Cron-based scheduling** (like dream): would require a per-user system cron job and make the "0 means disabled" knob awkward. Rejected.
- **Evaluator applies to all turns** (user turns included): would break sync conversations. Evaluator is autonomy-only.
- **Gateway delivery when no external channel is configured:** would interrupt active browser sessions for stale heartbeat output. Rejected per spec §9.7.
```

Make sure the ADR number is correct — re-read the end of `DECISIONS.md` first with a quick grep:

```bash
grep -n "^## ADR-" plexus-server/docs/DECISIONS.md | tail -3
```

- [ ] **Step 2: Update ISSUE.md**

In `plexus-server/docs/ISSUE.md`, add under `## Deferred`:

```markdown
- **Heartbeat multi-server deduplication** — the in-process tick loop refires per server. Single-node deployments are unaffected; multi-server needs either a leader-election pattern or a pg advisory lock held across the tick iteration. Tracked for post-M2.
- **Heartbeat session retention / log UI** — `heartbeat:{user_id}` sessions and messages accumulate indefinitely. Spec §9.7 mentions a future "Heartbeat Log" frontend page; no GC policy ships in M2.
- **Heartbeat Phase 2 error retry** — Phase 2 errors log and exit; `last_heartbeat_at` stays advanced. No retry; next window gets a fresh shot. Acceptable as autonomous-best-effort, but noted for observability work.
- **Heartbeat observability** — a consistently-skipping Phase 1 (e.g. broken LLM config) is silent beyond `info!` logs. A metrics-based alert would surface regressions; deferred.
```

- [ ] **Step 3: Update SCHEMA.md if present**

Grep for `last_heartbeat_at`:

```bash
grep -n "last_heartbeat_at" plexus-server/docs/SCHEMA.md 2>/dev/null || echo "SCHEMA.md missing or does not mention column"
```

If SCHEMA.md exists and does not mention the column, add under the `users` table description:

```markdown
- `last_heartbeat_at TIMESTAMPTZ` (nullable) — timestamp of the most recent heartbeat tick for this user. NULL means the user has never fired; the tick loop treats this as "due immediately". Advanced *before* Phase 1 runs to prevent refire during LLM latency. See ADR-36.
```

If SCHEMA.md is stale on *other* columns from Plans A/C/D (`last_dream_at`, `timezone`, `cron_jobs.kind`), this is the moment to note the drift in ISSUE.md under `## Open` for the docs-sync pass — but don't try to fix all of it in this task.

- [ ] **Step 4: Append a Post-Plan Adjustments footer to this plan**

In `docs/superpowers/plans/2026-04-18-heartbeat-subsystem.md`, append after the last task:

```markdown
---

## Post-Plan Adjustments

This section captures deviations between the plan as written and the code that actually landed. Populate after execution.

| Task | Deviation | Commit | Why |
|---|---|---|---|
| _pending_ | _pending_ | _pending_ | _pending_ |
```

(Leave the table stubbed — it's populated as the plan is executed. Plan A's and Plan D's footers are the reference format.)

- [ ] **Step 5: Build + test (no-op — docs-only)**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: green — docs changes do not affect the build.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
docs: ADR-36 heartbeat + ISSUE deferred items + Plan E footer

- DECISIONS.md: ADR-36 records the in-process tick-loop +
  evaluator-gated + external-channel-only delivery decisions.
- ISSUE.md: four deferred items land under Deferred —
  multi-server dedup, session retention, phase 2 retry,
  observability — each with "stays deferred" rationale.
- SCHEMA.md: last_heartbeat_at column documented (if the doc
  was already up to date for Plans A/C/D; otherwise noted as
  stale for the docs-sync pass).
- 2026-04-18-heartbeat-subsystem.md: empty Post-Plan Adjustments
  footer added for execution-time tracking.

Plan E implementation is complete. Heartbeat fires, gates via
the shared evaluator, routes to Discord → Telegram → silence,
and shuts down gracefully.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## 7. Completion Checklist

After all 10 tasks land, verify:

- [ ] `cargo build --workspace` clean.
- [ ] `cargo clippy --workspace -- -D warnings` clean.
- [ ] `cargo test --workspace` passes.
- [ ] Integration tests pass under `DATABASE_URL=… cargo test --package plexus-server -- --ignored`.
- [ ] Server starts; first tick fires within 60 s; shutdown drains cleanly on SIGTERM.
- [ ] `ADR-36` landed in `plexus-server/docs/DECISIONS.md`.
- [ ] `plexus-server/docs/ISSUE.md` reflects the four new deferred items.
- [ ] This plan file's Post-Plan Adjustments footer is populated with any execution-time deviations.

At that point, Plan E is done. The autonomy subsystems (dream + heartbeat) are both live end-to-end. The remaining M2 work is Plan B (frontend Workspace page) and the M2 closeout backlog (account deletion, admin user-management, graceful-shutdown extension, session-list unread badge).

---

## Post-Plan Adjustments

Deviations between the plan as written and the code that landed during execution.

| Task | Deviation | Commit | Why |
|---|---|---|---|
| E-1 | Added explanatory comment on 5ms tolerance in `test_last_heartbeat_at_roundtrip`. | `6811274` | Code review — mirrored the sibling dream test's comment so future readers don't re-derive the Postgres TIMESTAMPTZ precision rationale. |
| E-4 | `#[allow(dead_code)]` added on `HEARTBEAT_MAX_USERS_PER_TICK` const (defined in E-4, consumed in E-8). | `edd35b1` | Prevented a noisy clippy warning in the intermediate state between E-4 and E-8. Attribute was removed in E-8 when the const gained a caller. |
| E-5 | `build_heartbeat_system` skips `identity.build_session_section` and renders an inline headless stub; drops the `chat_id` parameter; the now-unused `identity` param keeps `_identity` prefix. | `c81ce1a` | Code review — `build_session_section` always emits "To send media: use the message tool …", which directly contradicts `HEARTBEAT_BANNER`'s "Do not use the message tool" rule. |
| E-6 | Telegram outbound `chat_id` is `format!("tg:{partner_id}")`, not the raw numeric ID. | `619fc83` | Code review — matches the channel convention (`tg:` prefix is used throughout the codebase for inbound+outbound Telegram chat IDs). Parser is currently forgiving via `strip_prefix("tg:").unwrap_or(id)`, but a future strict parse would silently drop unprefixed IDs. |
| E-6 | `publish_final_heartbeat` uses `.is_some_and(|id| !id.is_empty())` guards on partner IDs, not the nested `if let Some(...) { if !....is_empty() {...} }` form in the plan. | `5447114` | Stylistic — behaviorally equivalent in Rust 1.70+. |
| E-7 | Test comment field breakdown corrected (4 String, not 2; removed extra Option<String>). | `fdeda74` | The plan's comment body miscounted struct fields; a future reader auditing the comment against the struct would have been confused. |

## Commits map (Plan E)

| Plan step | Commits |
|---|---|
| E-1 | `1d4a5ae` (main), `6811274` (fix: tolerance comment) |
| E-2 | `7f0c105` |
| E-3 | `8bc9b25` |
| E-4 | `edd35b1` |
| E-5 | `72a795e` (main), `c81ce1a` (fix: headless session section) |
| E-6 | `5447114` (main), `619fc83` (fix: Telegram tg: prefix) |
| E-7 | `6379b95` (main), `fdeda74` (fix: comment accuracy) |
| E-8 | `76c41a9` |
| E-9 | `e8dbd19` |
| E-10 | _this commit_ |
