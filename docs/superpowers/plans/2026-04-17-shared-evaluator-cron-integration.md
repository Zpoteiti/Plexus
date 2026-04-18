# Shared Evaluator + Cron Integration Implementation Plan (Plan C of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Spec reference:** The full design lives at `/home/yucheng/Documents/GitHub/Plexus/docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md`. Read it if this plan's context seems incomplete — the spec carries the "why" behind every choice.
>
> **This is Plan C of 5.** Plan A (workspace foundation) landed over commits e6f1da4..2fe90a0 (all 21 tasks green). Remaining: **D** dream subsystem (consumes this plan's evaluator), **E** heartbeat subsystem (same), **B** frontend Workspace page (user-visible layer).

**Goal:** Build the shared post-run evaluator that gates autonomous-agent notifications (cron, later heartbeat) behind an LLM-decided "is this worth interrupting the user?" check, and protect system-owned cron jobs from user deletion so Plan D can register "dream" as a protected cron job.

**Architecture:** A new `evaluator.rs` module exposes one function — `evaluate_notification(state, user_id, final_message, purpose) -> EvaluationResult` — that makes a single cheap LLM call with a virtual `evaluate_notification` tool. The tool carries the `should_notify` decision + a reason string; default silence on any error. The agent loop's cron-completion path calls this function before publishing the `OutboundEvent`. Separately, both the `cron` server tool's `remove` action and the `DELETE /api/cron/{id}` HTTP handler learn to refuse `kind = 'system'` jobs so Plan D's "dream" registration stays permanent.

**Tech Stack:** Rust 1.85 (edition 2024), tokio, axum 0.7, sqlx (PostgreSQL), chrono + chrono_tz, serde_json, tracing. Reuses the existing `providers::openai::call_llm` + `LlmResponse` plumbing.

**Parent branch:** current `M3-gateway-frontend`, based on commit `2fe90a0` (A-21).

---

## 1. Overview

Three things happen in this plan:

1. **Evaluator module.** New `plexus-server/src/evaluator.rs` module. One public function that takes an assistant's final message, a purpose label, and the user_id; returns `{should_notify, reason}`. Injects the user's local time (from `users.timezone`, added by A-2 and read via `db::users::get_timezone` added by A-17) so the LLM can make "is it 4 AM?" judgments. Default-silent on any error path.

2. **Cron integration.** Agent loop currently publishes the final `OutboundEvent` unconditionally when a ReAct turn finishes. This plan adds a branch: when the event was cron-driven, check `cron_jobs.deliver`. If `false`: skip publish. If `true`: run the evaluator. Only publish when `should_notify: true`. User-driven turns and (eventually) heartbeat turns flow through the same code path later plans plug into.

3. **System-cron protection.** `cron_jobs.kind` column was added by A-2 with values `'user' | 'system'`. Plan D will create a `kind='system'` row named "dream" per user at registration. This plan teaches the `cron` tool's `remove` action and the `DELETE /api/cron/{id}` endpoint to refuse system jobs, so users can't accidentally or deliberately nuke the dream scheduler.

Plans D and E both consume the evaluator from #1. Plan D depends on protection from #3.

## 2. Goals & Non-Goals

**Goals**

- Introduce a single reusable evaluator helper that both cron and heartbeat will call post-agent-turn.
- Gate cron delivery behind the evaluator when `deliver = true`.
- Preserve the `deliver = false` escape hatch — cron jobs that only write files or edit state, never ping the user.
- Refuse removal of system-owned cron jobs through both the tool and the HTTP endpoint.
- Consume the currently-unused `db::users::get_timezone` helper (A-17's dead-code warning goes away).

**Non-Goals**

- Adding `EventKind` enum to `InboundEvent`. Current `cron_job_id.is_some()` discriminator is sufficient for the cron-evaluator wiring. Plans D/E will introduce `EventKind` when they add their own dispatch paths (per §11.1 of the spec).
- Per-user evaluator-model overrides. The evaluator uses the global `state.llm_config` just like the main agent loop.
- An admin UI for toggling `dream_enabled` or managing system jobs. `system_config.dream_enabled` is edited via existing admin plumbing (psql or a future admin page).
- Notification templating, batching, or channel fan-out. The evaluator's output goes through the existing `OutboundEvent` path.
- Tests that stand up a real LLM. Evaluator tests mock `call_llm` or exercise the error-path (default silence) exclusively.

## 3. Design

### 3.1 Evaluator shape

```rust
// plexus-server/src/evaluator.rs

pub struct EvaluationResult {
    pub should_notify: bool,
    pub reason: String,
}

pub async fn evaluate_notification(
    state: &std::sync::Arc<crate::state::AppState>,
    user_id: &str,
    final_message: &str,
    purpose: &str,
) -> EvaluationResult;
```

- `purpose` is a short human-written label like `"cron job 'daily-standup'"` or `"heartbeat wake-up"`. Injected into the system prompt so the LLM understands *why* this message was produced.
- `final_message` is the assistant's last user-visible text (post-think-tag stripping — what would have gone to the channel).
- The function never panics. On any error (DB, LLM call, parse failure, empty tool_calls), it returns `should_notify: false` with a diagnostic reason string and logs a `warn!`.

### 3.2 Evaluator LLM call

Single round-trip through `providers::openai::call_llm`:

- **System prompt:** short, fixed string. "Decide whether to ping the user now…" — full text in task C-1.
- **User message:** contains `purpose`, the user's current local time (computed from `users.timezone` via `chrono_tz`), and `final_message`. No conversation history — evaluator is stateless.
- **Tool:** virtual `evaluate_notification(should_notify: bool, reason: string)` injected inline. Not registered in the global tool registry. Not dispatched. This is nanobot's "decision oracle" pattern.
- **tool_choice:** `"required"` — force the LLM to call the tool rather than emit free-form text.

The existing `call_llm(client, config, messages, tools)` signature accepts `Option<String>` for `tool_choice` via the `CompletionRequest` struct. Check whether the current signature exposes that; if not, extend it (small change — the private struct already has the field at `openai.rs:171`).

### 3.3 Timezone handling

`users.timezone` is a TEXT column, default `'UTC'`, added by A-2. `db::users::get_timezone(pool, user_id) -> Result<String, sqlx::Error>` exists as of A-17 but currently has no caller (the dead-code warning).

The evaluator parses the string with `chrono_tz::Tz::from_str` and applies it to `chrono::Utc::now()`. Malformed or unknown timezone strings fall back to UTC with a `warn!` log.

`chrono_tz` is already a dep per the cron code's timezone validation (see `cron_tool::compute_next_cron_pub`).

### 3.4 Cron-side wiring

Current agent loop at `agent_loop.rs:237-250` publishes the final `OutboundEvent` unconditionally:

```rust
let _ = state.outbound_tx.send(OutboundEvent { ... }).await;
return Ok(());
```

Replace the `OutboundEvent` construction with a helper that consults:

1. Is this a cron turn? (`event.cron_job_id.is_some()`)
2. If yes: load the cron job row (`db::cron::find_by_id`). Check `deliver`:
   - `deliver == false`: skip publish. Log at `info`.
   - `deliver == true`: call `evaluator::evaluate_notification(state, user_id, content, purpose)`. Publish only if `should_notify`.
3. If no (user turn, or future heartbeat — heartbeat plan will wire its own branch): publish as today.

The helper lives in `agent_loop.rs` (or a tiny `delivery.rs` sibling) and encapsulates the branch so later turn-types can add their own paths without duplicating.

### 3.5 System-cron protection

Two touch points, both read `cron_jobs.kind` before mutating:

- **`cron` server tool (`server_tools/cron_tool.rs`).** The `remove` action currently loads the job by `job_id` + `user_id` and calls `db::cron::delete_job`. Add a `kind` check between load and delete: if `kind == "system"`, return `(1, "Cannot remove system cron jobs (e.g. 'dream'). These are managed by the server.")`.
- **`DELETE /api/cron/{id}` handler (`auth/cron_api.rs`).** Same guard: 403 with a similar message when the job's `kind == "system"`.

Both paths must load the job to check `kind`, so there's one extra query per delete — acceptable (deletes are rare).

### 3.6 Helper for Plan D

Plan D will need to create a system cron job for each user during registration. Expose a reusable helper now so D doesn't have to duplicate the insert logic:

```rust
// plexus-server/src/db/cron.rs

/// Create a system-owned cron job. Idempotent: if a job with the same
/// (user_id, name, kind='system') tuple already exists, this is a no-op.
/// Used by Plan D to register the per-user "dream" job.
pub async fn ensure_system_cron_job(
    pool: &PgPool,
    user_id: &str,
    name: &str,
    cron_expr: &str,
    timezone: &str,
    message: &str,
    channel: &str,
    chat_id: &str,
    deliver: bool,
) -> Result<(), sqlx::Error>;
```

Plan D will call this from `workspace::registration::initialize_user_workspace` (or a new sibling function; Plan D decides). Plan C just ships the helper + a test; no caller lands until D.

### 3.7 Error messages

Evaluator errors → `warn!` with structured fields (`user_id`, `purpose`, error). Return `should_notify: false` with `reason: "evaluator error: {e}"` so downstream logs / audit trails capture why a notification was suppressed.

System-cron removal errors use a consistent phrase:

> "Cannot remove system cron jobs (these are managed by the server)."

so tests can assert a substring that both paths share.

## 4. File Structure

### New files

| File | Responsibility |
|---|---|
| `plexus-server/src/evaluator.rs` | Public `evaluate_notification(state, user_id, final_message, purpose) -> EvaluationResult`. Module-local virtual-tool definition + LLM call + fallback behavior. |
| `plexus-server/src/evaluator/tests.rs` *(optional, inline under `#[cfg(test)] mod tests`)* | Unit tests for the fallback-silence path and purely-parsing branches. |

### Modified files

| File | Change |
|---|---|
| `plexus-server/src/main.rs` | Add `pub mod evaluator;` declaration. |
| `plexus-server/src/agent_loop.rs` | Replace unconditional final-publish with a helper that branches on `cron_job_id` and consults the evaluator when `deliver = true`. |
| `plexus-server/src/server_tools/cron_tool.rs` | `remove` action loads the job, checks `kind`, refuses if `system`. |
| `plexus-server/src/auth/cron_api.rs` | `DELETE /api/cron/{id}` handler checks `kind`, returns 403 if `system`. |
| `plexus-server/src/db/cron.rs` | Add `ensure_system_cron_job` helper for Plan D. |
| `plexus-server/src/providers/openai.rs` *(only if needed)* | Extend `call_llm` signature to accept `tool_choice: Option<String>` if it doesn't already — required for forcing the evaluator's tool call. Tiny, backward-compatible change. |

### Tests

| File | Scope |
|---|---|
| `plexus-server/src/evaluator.rs` (inline `#[cfg(test)] mod tests`) | Default-silence on error paths; reason string populated; parsing when a well-formed tool call arrives. |
| `plexus-server/src/server_tools/cron_tool.rs` (inline) | `remove` on a `kind='system'` job returns exit 1 with the expected message; `remove` on a `kind='user'` job still succeeds. |
| `plexus-server/src/auth/cron_api.rs` (inline or existing test module) | `DELETE /api/cron/{id}` returns 403 for system jobs, 200/204 for user jobs. |
| `plexus-server/src/db/cron.rs` (inline) | `ensure_system_cron_job` creates a row; second call with the same (user_id, name) is a no-op (idempotent). |
| `plexus-server/tests/cron_evaluator_integration.rs` *(optional, can be deferred to Plan D)* | End-to-end: cron turn with `deliver=true`, evaluator returns silence → no OutboundEvent published. |

## 5. Testing Strategy

- **Inline unit tests** for the evaluator's error paths (missing timezone, LLM-call failure simulated by pointing at an unreachable URL in `test_minimal` state, malformed tool-call args).
- **`#[cfg(test)]` mocking of `call_llm`:** prefer threading a trait or function pointer through the module if the refactor is cheap; otherwise exercise the error branches via DB or config sabotage and skip the happy-path LLM test (deferred to end-to-end with a real model in staging).
- **System-cron protection** tests run against a real sqlx pool (`#[tokio::test]` with a test DB, same pattern that files like `db/cron.rs` already use — check existing test style). Create a row with `kind='system'`, call the delete path, assert refusal; create a `kind='user'` row, assert success.
- **`ensure_system_cron_job` idempotency** tests do a create + create + count-is-1 sanity check.

## 6. Tasks

Five tasks, all TDD. Each ends with one commit. Plan A's Post-Plan Adjustments footer contract applies here too — if any task deviates materially from the plan text, append a row to the footer.

---

### Task C-1: evaluator.rs module

**Files:**
- Create: `plexus-server/src/evaluator.rs`
- Modify: `plexus-server/src/main.rs` (add `pub mod evaluator;`)
- Modify: `plexus-server/src/providers/openai.rs` *(only if `call_llm` doesn't already accept `tool_choice`)*

- [ ] **Step 1: Verify / extend `call_llm` to accept `tool_choice`**

Inspect `providers/openai.rs:200`. The `CompletionRequest` struct already has a `tool_choice: Option<String>` field (line ~171), but the public `call_llm` function may not take it. If not, widen the signature:

```rust
pub async fn call_llm(
    client: &reqwest::Client,
    config: &LlmConfig,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<Value>>,
    tool_choice: Option<String>,   // NEW — e.g. "required" to force a tool call
) -> Result<LlmResponse, String>
```

Update all existing call sites (grep `call_llm`) to pass `None` for `tool_choice`. This is a backward-compatible extension.

If the signature already exposes `tool_choice`, skip this step.

- [ ] **Step 2: Create `evaluator.rs` skeleton**

```rust
// plexus-server/src/evaluator.rs
//! Shared post-run evaluator for autonomous agent outputs (cron, heartbeat).
//!
//! Given an agent's final message and a purpose label, returns whether the
//! user should be pinged. The LLM sees the user's current local time so it
//! can reason about "is this a good time to interrupt?" — the 4 AM guard.
//!
//! Default silence on any error — silence is the safe failure mode.

use crate::state::AppState;
use serde::Deserialize;
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationResult {
    pub should_notify: bool,
    pub reason: String,
}

const SYSTEM_PROMPT: &str = "\
You are a notification evaluator. Given an autonomous agent's output, \
decide whether to ping the user now. Call the evaluate_notification tool \
with should_notify: true only if the user would genuinely benefit from \
seeing this message at the current time. Return false if the output is \
status-only, routine, or the user is likely sleeping (typical waking hours: \
8 AM to 10 PM local). When uncertain, default to silence.";

fn virtual_tool() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "evaluate_notification",
            "description": "Decide whether to ping the user about this autonomous output.",
            "parameters": {
                "type": "object",
                "properties": {
                    "should_notify": { "type": "boolean" },
                    "reason": { "type": "string" }
                },
                "required": ["should_notify"]
            }
        }
    })
}

#[derive(Deserialize)]
struct ToolArgs {
    should_notify: bool,
    #[serde(default)]
    reason: String,
}

pub async fn evaluate_notification(
    state: &Arc<AppState>,
    user_id: &str,
    final_message: &str,
    purpose: &str,
) -> EvaluationResult {
    // 1. Load timezone (A-2 column via A-17 helper). Default UTC on failure.
    let tz_string = crate::db::users::get_timezone(&state.db, user_id)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, user_id, "evaluator: timezone lookup failed, using UTC");
            "UTC".into()
        });
    let tz: chrono_tz::Tz = tz_string.parse().unwrap_or(chrono_tz::UTC);
    let local_now = chrono::Utc::now().with_timezone(&tz);

    // 2. Build the messages for the evaluator call.
    let user_body = format!(
        "## Purpose\n{purpose}\n\n## Current local time\n{}\n\n## Output to evaluate\n{final_message}",
        local_now.format("%A %H:%M %Z")
    );
    let messages = vec![
        crate::providers::openai::ChatMessage::system(SYSTEM_PROMPT),
        crate::providers::openai::ChatMessage::user(user_body),
    ];

    // 3. Load current LLM config. If unavailable, silence.
    let llm_config = match state.llm_config.read().await.clone() {
        Some(c) => c,
        None => {
            warn!(user_id, purpose, "evaluator: no LLM config available, defaulting to silence");
            return EvaluationResult {
                should_notify: false,
                reason: "no LLM config".into(),
            };
        }
    };

    // 4. Call the LLM, force tool use.
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
        Ok(other) => {
            warn!(user_id, purpose, ?other_kind = std::any::type_name_of_val(&other), "evaluator: LLM did not return a tool call, defaulting to silence");
            return EvaluationResult {
                should_notify: false,
                reason: "LLM did not call the evaluate_notification tool".into(),
            };
        }
        Err(e) => {
            warn!(error = %e, user_id, purpose, "evaluator: LLM call failed, defaulting to silence");
            return EvaluationResult {
                should_notify: false,
                reason: format!("evaluator LLM error: {e}"),
            };
        }
    };

    // 5. Parse the first tool call. Ignore any additional (defensive).
    let first = &calls[0];
    if first.function.name != "evaluate_notification" {
        warn!(got = %first.function.name, "evaluator: unexpected tool name, defaulting to silence");
        return EvaluationResult {
            should_notify: false,
            reason: format!("unexpected tool name: {}", first.function.name),
        };
    }
    match serde_json::from_str::<ToolArgs>(&first.function.arguments) {
        Ok(args) => EvaluationResult {
            should_notify: args.should_notify,
            reason: args.reason,
        },
        Err(e) => {
            warn!(error = %e, args = %first.function.arguments, "evaluator: failed to parse tool args, defaulting to silence");
            EvaluationResult {
                should_notify: false,
                reason: format!("parse error: {e}"),
            }
        }
    }
}
```

Check the actual field name on `AppState` — in A-7's `test_minimal` I saw `http_client` used. If it's named differently (`client`, `reqwest`, etc.), match the real name. Similarly, `state.llm_config` may be `RwLock<Option<LlmConfig>>` already per ADR-5; confirm the lock shape and adjust `read().await.clone()` accordingly.

- [ ] **Step 3: Wire `pub mod evaluator;` into main.rs**

Find the block of `pub mod …;` declarations in `plexus-server/src/main.rs` (near `pub mod workspace;` / `pub mod skills_cache;` from Plan A) and add:

```rust
pub mod evaluator;
```

- [ ] **Step 4: Inline unit tests for error branches**

Add at the bottom of `plexus-server/src/evaluator.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_default_silence_when_no_llm_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = crate::state::AppState::test_minimal(tmp.path());
        // test_minimal does not set an LLM config.
        let result = evaluate_notification(&state, "alice", "Report produced.", "cron 'daily'").await;
        assert!(!result.should_notify);
        assert!(result.reason.contains("no LLM config") || result.reason.contains("LLM"));
    }

    #[test]
    fn test_virtual_tool_shape() {
        let tool = virtual_tool();
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "evaluate_notification");
        let params = &tool["function"]["parameters"];
        assert_eq!(params["properties"]["should_notify"]["type"], "boolean");
        assert_eq!(params["required"], serde_json::json!(["should_notify"]));
    }

    #[test]
    fn test_tool_args_parse_accepts_missing_reason() {
        // reason has #[serde(default)] so missing field -> empty string.
        let parsed: ToolArgs = serde_json::from_str(r#"{"should_notify": true}"#).unwrap();
        assert!(parsed.should_notify);
        assert_eq!(parsed.reason, "");
    }

    #[test]
    fn test_tool_args_parse_rejects_missing_should_notify() {
        let err = serde_json::from_str::<ToolArgs>(r#"{"reason": "ok"}"#);
        assert!(err.is_err());
    }
}
```

- [ ] **Step 5: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server evaluator
```

Expected: 4 tests pass (one async, three sync). Build clean (no new warnings — `update_timezone`/`get_timezone` were previously dead-code-warned in A-17; `get_timezone` is now consumed by the evaluator, so one warning goes away).

- [ ] **Step 6: Commit**

```bash
git add plexus-server/src/evaluator.rs plexus-server/src/main.rs plexus-server/src/providers/openai.rs
git commit -m "feat: shared evaluate_notification evaluator module

Single LLM call that decides whether an autonomous agent's final
message is worth pinging the user about. Takes user_id (for
timezone), final_message, and a purpose label ('cron job ...'
or 'heartbeat wake-up'). Uses a virtual evaluate_notification
tool injected only for this call — not in the global tool
registry.

Default silence on every error path: no LLM config, LLM call
failure, malformed response, unexpected tool. Silence is the
safe failure mode for notification decisions.

Consumes db::users::get_timezone (A-17 added it; currently
dead-code) to give the LLM the user's local time so it can
reason about 4 AM vs 4 PM.

call_llm signature extended with an optional tool_choice so
callers can force a tool call (\"required\"). Existing callers
pass None and are unchanged.

Plans D (dream) and E (heartbeat) consume this helper for their
post-run notification gating."
```

---

### Task C-2: Cron delivery integration in agent_loop

**Files:**
- Modify: `plexus-server/src/agent_loop.rs` (final-publish path around line 237-250)

- [ ] **Step 1: Write a failing test (acceptance scaffold)**

The agent loop's final-publish path is deep inside `handle_event` — hard to unit-test in isolation. Write an integration-style test that exercises the new cron-delivery helper directly. Factor the branch logic into a pub(crate) helper first, then test the helper.

Add to the `#[cfg(test)] mod tests` block in `agent_loop.rs` (create one if missing):

```rust
#[cfg(test)]
mod deliver_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_publish_final_skips_when_cron_deliver_false() {
        let tmp = TempDir::new().unwrap();
        let (state, mut rx) = crate::state::AppState::test_minimal_with_outbound(tmp.path());

        // Pretend this is a cron event whose job has deliver=false.
        let params = PublishFinalParams {
            channel: "gateway".into(),
            chat_id: Some("chat-1".into()),
            session_id: "cron:job-1".into(),
            user_id: "alice".into(),
            content: "all done".into(),
            cron_job_id: Some("job-1".into()),
            job_deliver: Some(false),
        };
        publish_final(&state, params).await;

        // Nothing should be published.
        assert!(rx.try_recv().is_err(), "deliver=false cron should not publish");
    }

    #[tokio::test]
    async fn test_publish_final_publishes_user_turn() {
        let tmp = TempDir::new().unwrap();
        let (state, mut rx) = crate::state::AppState::test_minimal_with_outbound(tmp.path());

        let params = PublishFinalParams {
            channel: "gateway".into(),
            chat_id: Some("chat-1".into()),
            session_id: "sess-1".into(),
            user_id: "alice".into(),
            content: "hi".into(),
            cron_job_id: None,
            job_deliver: None,
        };
        publish_final(&state, params).await;

        let event = rx.recv().await.expect("user turn must publish");
        assert_eq!(event.content, "hi");
    }
}
```

- [ ] **Step 2: Define the helper shape + params struct**

At module scope in `agent_loop.rs`, add above (or near) `handle_event`:

```rust
pub(crate) struct PublishFinalParams {
    pub channel: String,
    pub chat_id: Option<String>,
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    /// None for user turns; Some(job_id) for cron-driven turns.
    pub cron_job_id: Option<String>,
    /// Only consulted when `cron_job_id` is `Some(_)`. When the caller
    /// has already loaded the cron row (to save a DB query), they pass
    /// the `deliver` flag here. When `None` and `cron_job_id` is set,
    /// the helper loads the job from DB.
    pub job_deliver: Option<bool>,
}

/// Final-message delivery branch.
///
/// - User turn (`cron_job_id == None`): publish OutboundEvent as today.
/// - Cron turn with `deliver == false`: skip publish (pure side-effect cron job).
/// - Cron turn with `deliver == true`: run the evaluator; publish only if
///   it returns `should_notify: true`. Silence on evaluator error.
///
/// Heartbeat turns (Plan E) will add a third branch that also calls the
/// evaluator with a different `purpose` label.
pub(crate) async fn publish_final(
    state: &std::sync::Arc<crate::state::AppState>,
    params: PublishFinalParams,
) {
    let PublishFinalParams {
        channel, chat_id, session_id, user_id, content, cron_job_id, job_deliver,
    } = params;

    // Decide whether to deliver.
    let should_deliver = match &cron_job_id {
        None => true,  // user turn
        Some(job_id) => {
            let deliver = match job_deliver {
                Some(d) => d,
                None => {
                    // Caller didn't pass the flag; load the job.
                    match crate::db::cron::find_by_id(&state.db, job_id).await {
                        Ok(Some(job)) => job.deliver,
                        _ => {
                            tracing::warn!(job_id, "publish_final: cron job lookup failed, defaulting to silence");
                            false
                        }
                    }
                }
            };

            if !deliver {
                tracing::info!(job_id, "cron deliver=false; skipping OutboundEvent publish");
                return;
            }

            // deliver == true: gate through the evaluator.
            let purpose = format!("cron job '{job_id}'");
            let eval = crate::evaluator::evaluate_notification(state, &user_id, &content, &purpose).await;
            if !eval.should_notify {
                tracing::info!(job_id, reason = %eval.reason, "evaluator suppressed cron delivery");
                return;
            }
            true
        }
    };

    if should_deliver {
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
}
```

- [ ] **Step 3: Replace the inline publish in `handle_event`**

At `agent_loop.rs:237-248`, replace:

```rust
let _ = state
    .outbound_tx
    .send(OutboundEvent {
        channel: event.channel.clone(),
        chat_id: event.chat_id.clone(),
        session_id: session_id.to_string(),
        user_id: user_id.to_string(),
        content: content,
        media: vec![],
    })
    .await;

return Ok(());
```

with:

```rust
publish_final(
    state,
    PublishFinalParams {
        channel: event.channel.clone(),
        chat_id: event.chat_id.clone(),
        session_id: session_id.to_string(),
        user_id: user_id.to_string(),
        content,
        cron_job_id: event.cron_job_id.clone(),
        job_deliver: None,  // helper loads from DB
    },
)
.await;

return Ok(());
```

- [ ] **Step 4: Run tests**

```bash
cargo test --package plexus-server agent_loop::deliver_tests
cargo test --package plexus-server  # full suite, confirm no regressions
```

Expected: 2 new tests pass. Full suite still green (~121 tests).

- [ ] **Step 5: Commit**

```bash
git add plexus-server/src/agent_loop.rs
git commit -m "feat: gate cron OutboundEvent publish through the evaluator

Extracts the final-message publish into a pub(crate) publish_final
helper. For user turns, publishes unconditionally (unchanged
behavior). For cron turns, consults cron_jobs.deliver:

- deliver = false: skip publish (pure side-effect cron job)
- deliver = true: run evaluator::evaluate_notification; publish
  only when the evaluator returns should_notify: true

Fixes the long-standing 'cron spam' problem: before this commit
every cron turn published its final assistant message to the
target channel regardless of whether the user would benefit from
seeing it. The evaluator injects the user's local time so 4 AM
status updates are suppressed by default.

Heartbeat turns (Plan E) will plug into this same helper when
they land; the API already accommodates a third branch."
```

---

### Task C-3: System-cron protection in the `cron` tool

**Files:**
- Modify: `plexus-server/src/server_tools/cron_tool.rs`

- [ ] **Step 1: Inspect the current `remove` action**

Open `plexus-server/src/server_tools/cron_tool.rs`. Find the match arm that handles `action == "remove"`. It currently loads the job, verifies user ownership, and calls `db::cron::delete_job`. Capture the current shape before editing.

- [ ] **Step 2: Add the `kind` check**

After the existing ownership load (the `db::cron::find_by_id` call), before the delete, insert:

```rust
if job.kind == "system" {
    return (1, "Cannot remove system cron jobs (these are managed by the server).".into());
}
```

If the struct field hasn't been added yet, first update `db::cron::CronJob` to include `pub kind: String`. The schema column exists as of A-2 so the sqlx query can read it — add to the struct and to any `SELECT` that returns `CronJob`.

- [ ] **Step 3: Test — system job rejects**

Add to the `#[cfg(test)] mod tests` block in `cron_tool.rs`:

```rust
#[tokio::test]
async fn test_remove_refuses_system_job() {
    // Use the sqlx-test pattern consistent with other DB-touching tests
    // in this project. Check an existing test for the exact macro/helper.
    // Skeleton:
    let pool = crate::db::test_pool().await;
    crate::db::users::create_user(&pool, "alice", "a@b.c", "", false).await.unwrap();
    crate::db::cron::ensure_system_cron_job(
        &pool, "alice", "dream", "0 */2 * * *", "UTC", "",
        "gateway", "-", false,
    ).await.unwrap();
    let job = crate::db::cron::list_by_user(&pool, "alice").await.unwrap()
        .into_iter().find(|j| j.name == "dream").unwrap();

    let state = /* ... construct a minimal AppState sharing this pool ... */;
    let ctx = ToolContext { /* fill */ };
    let args = serde_json::json!({"action": "remove", "job_id": job.job_id});
    let (code, out) = cron(&state, &ctx, &args).await;
    assert_eq!(code, 1);
    assert!(out.contains("system"));

    // Confirm the job is still there.
    let still = crate::db::cron::find_by_id(&pool, &job.job_id).await.unwrap();
    assert!(still.is_some());
}
```

Adapt the test scaffold to whatever pattern `cron_tool.rs` already uses (if any). If the file has no DB-integrated tests today, consider whether an integration test under `plexus-server/tests/` is cleaner. The key invariant is: `remove` on a system job must return exit code 1 with a message containing "system", and the row must survive.

- [ ] **Step 4: Test — user job still succeeds**

Parallel test where `kind='user'` (the default) to prove the check doesn't over-refuse.

- [ ] **Step 5: Run tests, commit**

```bash
cargo test --package plexus-server cron_tool
```

```bash
git add plexus-server/src/server_tools/cron_tool.rs plexus-server/src/db/cron.rs
git commit -m "feat: cron tool refuses to remove system jobs

The cron_jobs.kind column (added in A-2) distinguishes user-
created jobs from server-managed ones. Plan D will register a
'dream' system job per user at registration. This commit teaches
the 'remove' action on the cron tool to refuse delete when
kind='system' with a clear error message, so users cannot
accidentally (or deliberately) nuke the dream scheduler.

kind=user jobs continue to delete normally."
```

---

### Task C-4: System-cron protection in the HTTP endpoint

**Files:**
- Modify: `plexus-server/src/auth/cron_api.rs`

- [ ] **Step 1: Find the DELETE handler**

`grep -n "DELETE\|delete_cron\|remove_cron" plexus-server/src/auth/cron_api.rs`. The handler is likely called `delete_cron` or similar and matched to `.route("/api/cron/{id}", delete(...))` somewhere in the auth module.

- [ ] **Step 2: Add the `kind` check before delete**

Load the job first (reuse `db::cron::find_by_id`), check `kind`:

```rust
let job = match crate::db::cron::find_by_id(&state.db, &job_id).await {
    Ok(Some(j)) => j,
    Ok(None) => return Err(ApiError::new(ErrorCode::NotFound, "Cron job not found")),
    Err(e) => return Err(ApiError::new(ErrorCode::InternalError, format!("DB: {e}"))),
};

if job.user_id != c.sub {
    return Err(ApiError::new(ErrorCode::Forbidden, "Not your cron job"));
}

if job.kind == "system" {
    return Err(ApiError::new(
        ErrorCode::Forbidden,
        "Cannot remove system cron jobs (these are managed by the server).",
    ));
}

crate::db::cron::delete_job(&state.db, &job_id, &c.sub).await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("Delete: {e}")))?;
```

Adapt to the actual claims variable name (`c` / `claims`) and error constructor.

- [ ] **Step 3: Integration test**

Add to `plexus-server/src/auth/cron_api.rs` `#[cfg(test)]` block or to the existing integration tests:

```rust
#[tokio::test]
async fn test_delete_cron_refuses_system_job() {
    // Build a test server with sqlx pool, create a user, create a
    // system cron job, send DELETE /api/cron/{id}, assert 403.
    // Use whatever test harness the other cron_api tests use.
    // Expected response body contains "system".
}
```

If no similar DELETE test exists today, the file may need a new harness. Follow the pattern used in `account-deletion` plan's tests (once that plan runs) or stick to a minimal `axum::Router::oneshot` test.

- [ ] **Step 4: Run tests, commit**

```bash
cargo test --package plexus-server cron_api
```

```bash
git add plexus-server/src/auth/cron_api.rs
git commit -m "feat: DELETE /api/cron/{id} refuses system jobs

Mirror of Task C-3 for the HTTP endpoint. Returns 403 Forbidden
with a message containing 'system' when the job's kind='system'.
Protects Plan D's 'dream' registration from external deletion via
curl or admin-panel callers that might bypass the cron tool."
```

---

### Task C-5: `ensure_system_cron_job` helper for Plan D

**Files:**
- Modify: `plexus-server/src/db/cron.rs`

- [ ] **Step 1: Write failing idempotency test**

Add to the `#[cfg(test)] mod tests` block in `db/cron.rs`:

```rust
#[sqlx::test]  // use whatever test macro db/cron.rs already uses; if none,
               // follow the pattern from db/users.rs's tests.
async fn test_ensure_system_cron_job_is_idempotent(pool: PgPool) {
    crate::db::users::create_user(&pool, "alice", "a@b.c", "", false).await.unwrap();

    // First call creates.
    ensure_system_cron_job(
        &pool, "alice", "dream", "0 */2 * * *", "UTC",
        "", "gateway", "-", false,
    ).await.unwrap();
    let jobs1 = list_by_user(&pool, "alice").await.unwrap();
    assert_eq!(jobs1.iter().filter(|j| j.name == "dream").count(), 1);

    // Second call is a no-op.
    ensure_system_cron_job(
        &pool, "alice", "dream", "0 */2 * * *", "UTC",
        "", "gateway", "-", false,
    ).await.unwrap();
    let jobs2 = list_by_user(&pool, "alice").await.unwrap();
    assert_eq!(jobs2.iter().filter(|j| j.name == "dream").count(), 1);

    // The existing row is kind='system'.
    let dream = jobs2.iter().find(|j| j.name == "dream").unwrap();
    assert_eq!(dream.kind, "system");
}
```

If `#[sqlx::test]` isn't configured (A-6 revealed the macros feature may not be enabled), either:
- Mark the test `#[ignore]` with a comment noting it needs a live DB, OR
- Use whatever test-DB helper `db::cron` already uses (grep existing tests).

- [ ] **Step 2: Implement `ensure_system_cron_job`**

```rust
/// Create a system-owned cron job for a user. Idempotent: if a job with
/// the same (user_id, name, kind='system') tuple already exists, this is
/// a no-op and returns Ok.
///
/// Used by Plan D to register a 'dream' job per user at registration time
/// and by any future system-managed scheduler.
///
/// The returned job has kind='system' so both the cron tool and the HTTP
/// endpoint refuse to delete it (see Tasks C-3 and C-4).
pub async fn ensure_system_cron_job(
    pool: &PgPool,
    user_id: &str,
    name: &str,
    cron_expr: &str,
    timezone: &str,
    message: &str,
    channel: &str,
    chat_id: &str,
    deliver: bool,
) -> Result<(), sqlx::Error> {
    // Does a system job with this (user_id, name) already exist?
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT job_id FROM cron_jobs \
         WHERE user_id = $1 AND name = $2 AND kind = 'system' \
         LIMIT 1",
    )
    .bind(user_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    if existing.is_some() {
        return Ok(());
    }

    let job_id = uuid::Uuid::new_v4().to_string();
    // Compute initial next_run_at so the poller picks it up.
    // If compute fails, leave NULL — the poller will re-evaluate.
    let next_run_at = crate::server_tools::cron_tool::compute_next_cron_pub(cron_expr, timezone).ok();

    sqlx::query(
        "INSERT INTO cron_jobs \
         (job_id, user_id, name, kind, enabled, cron_expr, every_seconds, timezone, \
          message, channel, chat_id, delete_after_run, deliver, next_run_at, run_count) \
         VALUES ($1, $2, $3, 'system', TRUE, $4, NULL, $5, $6, $7, $8, FALSE, $9, $10, 0)",
    )
    .bind(&job_id)
    .bind(user_id)
    .bind(name)
    .bind(cron_expr)
    .bind(timezone)
    .bind(message)
    .bind(channel)
    .bind(chat_id)
    .bind(deliver)
    .bind(next_run_at)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 3: Run tests**

```bash
cargo build --package plexus-server
cargo test --package plexus-server db::cron
```

- [ ] **Step 4: Commit**

```bash
git add plexus-server/src/db/cron.rs
git commit -m "feat: ensure_system_cron_job helper for Plan D

Idempotent system-cron creator. Used by Plan D's dream
registration (runs at user-workspace initialization): ensures
each user has a 'dream' system cron job, but subsequent boots
or retries don't create duplicates.

System-kind jobs are protected from user deletion by C-3 and
C-4, so the invariant 'every user has exactly one dream job'
is cheap to maintain: call ensure_system_cron_job(...) at any
safe moment and it converges on the correct state."
```

---

## 7. Self-Review Checklist (run before declaring Plan C done)

1. **Spec coverage:**
   - §11.4 (shared evaluator module) → Task C-1.
   - §9.6 (heartbeat evaluator — same utility) → Task C-1 ships the utility; E consumes.
   - §9.6 bonus ("fixes current cron unconditional-delivery") → Task C-2.
   - §8.2 (system cron protection) → Tasks C-3, C-4, C-5.
   - §8.2's `ensure_system_cron_job` (used by Plan D) → Task C-5.

2. **Placeholder scan:** Search for "TBD"/"TODO"/"similar to" — none should appear outside explicit deferral comments pointing at Plans D or E.

3. **Type consistency:**
   - `EvaluationResult { should_notify: bool, reason: String }` — used consistently in C-1 and referenced by future plans.
   - `publish_final` + `PublishFinalParams` — introduced in C-2; Plan E will add a fourth branch using the same struct.
   - `ensure_system_cron_job(pool, user_id, name, cron_expr, timezone, message, channel, chat_id, deliver)` — 9 args; Plan D calls it with exactly this signature.
   - `job.kind == "system"` — consistent literal across C-3, C-4, C-5.

4. **No dangling consumers:** `get_timezone` is unused today. C-1 consumes it. `update_timezone` stays unconsumed — Plan B's Settings-page timezone editor will consume it. Acceptable gap.

## 8. Execution Hints

- Tasks C-1, C-3, C-4, C-5 are mostly independent and can be executed in any order — implementers working alone should still do them sequentially to keep commits small and reviews focused.
- Task C-2 depends on C-1 (evaluator module must exist before the agent loop calls it). Execute after C-1.
- Total commits: 5 (one per task). No expected fixup commits — the scope is narrow.
- Code-review focus areas: the silence-by-default error branches in C-1 (easy to accidentally make one of them `true`), and the `job.kind == "system"` string literal appearing in three places (consider lifting to a `db::cron::SYSTEM_KIND` const if reviewers push on it).
