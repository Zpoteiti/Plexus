# Cron Nanobot-Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix Plexus's cron scheduler to match nanobot's "wait-until-done-then-reschedule" design, eliminating overlapping runs, double-firing, and lost executions on crash.

**Architecture:** Introduce an atomic DB "claim" step (sets `next_run_at = NULL`, `claimed_at = NOW()`) that prevents any other server or poll cycle from re-queuing a running job. After the agent loop finishes its full ReAct turn for a cron event, it calls back into cron to compute and write the real next run time. A stuck-job recovery sweep handles crashes between claim and reschedule.

**Tech Stack:** Rust 1.85+, sqlx (PostgreSQL), tokio, plexus-server crate only.

---

## File Map

| File | Action | Change |
|------|--------|--------|
| `plexus-server/src/db/mod.rs` | Modify | Add `ALTER TABLE` migrations for `claimed_at` and `last_status` columns |
| `plexus-server/src/db/cron.rs` | Modify | Update `CronJob` struct; add `claim_due_jobs`, `find_by_id`, `reschedule_job`, `recover_stuck_jobs`, `unclaim_job`; update `disable_job`; remove `find_due_jobs`, `update_after_run` |
| `plexus-server/src/cron.rs` | Modify | Rewrite `poll_and_execute` (claim+recovery, no immediate reschedule); add public `reschedule_after_completion` |
| `plexus-server/src/agent_loop.rs` | Modify | Extract `cron_job_id` before moving event into `handle_event`; call `cron::reschedule_after_completion` after each event |

---

## Task 1: DB Schema — Add Columns and Migrations

**Files:**
- Modify: `plexus-server/src/db/mod.rs`
- Modify: `plexus-server/src/db/cron.rs`

### Context

The `cron_jobs` table needs two new columns:
- `claimed_at TIMESTAMPTZ` — set when the poller atomically grabs a job; cleared when the agent finishes rescheduling. A non-NULL value with age > 30 minutes means the server crashed mid-execution.
- `last_status TEXT` — mirrors nanobot's `last_status` field (`"ok"`, `"error"`, `"recovered"`). Useful for debugging and the cron list UI.

`CREATE TABLE IF NOT EXISTS` blocks can't add columns — we need `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` statements which sqlx runs idempotently.

---

- [ ] **Step 1.1: Write the failing compile-check test**

In `plexus-server/src/db/cron.rs`, add this test at the bottom. It will fail to compile until the struct has the new fields.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Proves CronJob struct has claimed_at and last_status fields.
    /// This is a compile-time check — if it builds, the fields exist.
    #[test]
    fn cron_job_struct_has_new_fields() {
        let j = CronJob {
            job_id: "test".into(),
            user_id: "u1".into(),
            name: "my job".into(),
            enabled: true,
            cron_expr: None,
            every_seconds: Some(60),
            timezone: "UTC".into(),
            message: "hello".into(),
            channel: "gateway".into(),
            chat_id: "chat1".into(),
            delete_after_run: false,
            deliver: true,
            next_run_at: None,
            last_run_at: None,
            run_count: 0,
            created_at: chrono::Utc::now(),
            claimed_at: None,
            last_status: None,
        };
        assert!(j.claimed_at.is_none());
        assert!(j.last_status.is_none());
    }
}
```

- [ ] **Step 1.2: Run to confirm it fails**

```bash
cd /home/yucheng/Documents/GitHub/Plexus
cargo test --package plexus-server db::cron::tests 2>&1 | tail -20
```

Expected: compile error mentioning `claimed_at` and `last_status` missing from struct.

- [ ] **Step 1.3: Add migrations to `db/mod.rs`**

Open `plexus-server/src/db/mod.rs`. Find the `statements` array in `create_tables()`. Add these two entries after the line that creates the `cron_jobs` table (after the closing `")`):

```rust
        // Migration: add claimed_at for atomic job claiming (cron nanobot-parity)
        "ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ",
        // Migration: add last_status for execution result tracking
        "ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS last_status TEXT",
```

- [ ] **Step 1.4: Update `CronJob` struct in `db/cron.rs`**

Replace the existing `CronJob` struct:

```rust
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CronJob {
    pub job_id: String,
    pub user_id: String,
    pub name: String,
    pub enabled: bool,
    pub cron_expr: Option<String>,
    pub every_seconds: Option<i32>,
    pub timezone: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    pub delete_after_run: bool,
    pub deliver: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub run_count: i32,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
}
```

- [ ] **Step 1.5: Run test to confirm struct compiles**

```bash
cargo test --package plexus-server db::cron::tests 2>&1 | tail -10
```

Expected: `test db::cron::tests::cron_job_struct_has_new_fields ... ok`

- [ ] **Step 1.6: Replace DB query functions in `db/cron.rs`**

Replace the entire contents of `plexus-server/src/db/cron.rs` below the struct with the following. This removes `find_due_jobs` and `update_after_run` (replaced by the new functions) and adds all new operations:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn create_job(
    pool: &PgPool,
    job_id: &str,
    user_id: &str,
    name: &str,
    cron_expr: Option<String>,
    every_seconds: Option<i32>,
    timezone: &str,
    message: &str,
    channel: &str,
    chat_id: &str,
    delete_after_run: bool,
    deliver: bool,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO cron_jobs \
         (job_id, user_id, name, cron_expr, every_seconds, timezone, message, \
          channel, chat_id, delete_after_run, deliver, next_run_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(job_id)
    .bind(user_id)
    .bind(name)
    .bind(cron_expr)
    .bind(every_seconds)
    .bind(timezone)
    .bind(message)
    .bind(channel)
    .bind(chat_id)
    .bind(delete_after_run)
    .bind(deliver)
    .bind(next_run_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_user(pool: &PgPool, user_id: &str) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>(
        "SELECT * FROM cron_jobs WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn find_by_id(pool: &PgPool, job_id: &str) -> Result<Option<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>("SELECT * FROM cron_jobs WHERE job_id = $1")
        .bind(job_id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_job(
    pool: &PgPool,
    job_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM cron_jobs WHERE job_id = $1 AND user_id = $2",
    )
    .bind(job_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Atomically claim all due jobs: sets next_run_at = NULL and claimed_at = NOW().
/// Returns the claimed jobs. Any job returned here is exclusively owned by this
/// server instance until reschedule_job / unclaim_job clears claimed_at.
/// Safe to call from multiple server nodes simultaneously — each UPDATE is atomic.
pub async fn claim_due_jobs(pool: &PgPool) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>(
        "UPDATE cron_jobs \
         SET claimed_at = NOW(), next_run_at = NULL \
         WHERE enabled = true \
           AND next_run_at IS NOT NULL \
           AND next_run_at <= NOW() \
         RETURNING *",
    )
    .fetch_all(pool)
    .await
}

/// Called by the agent loop after successfully completing a cron event turn.
/// Sets the next run time and clears claimed_at so the poller can pick it up again.
pub async fn reschedule_job(
    pool: &PgPool,
    job_id: &str,
    next_run_at: Option<DateTime<Utc>>,
    success: bool,
) -> Result<(), sqlx::Error> {
    let status = if success { "ok" } else { "error" };
    sqlx::query(
        "UPDATE cron_jobs \
         SET last_run_at = NOW(), \
             run_count = run_count + 1, \
             next_run_at = $1, \
             claimed_at = NULL, \
             last_status = $2 \
         WHERE job_id = $3",
    )
    .bind(next_run_at)
    .bind(status)
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Called when the bus fails to dispatch a claimed job.
/// Resets the job to retry in 1 minute instead of waiting for stuck recovery.
pub async fn unclaim_job(pool: &PgPool, job_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cron_jobs \
         SET claimed_at = NULL, \
             next_run_at = NOW() + INTERVAL '1 minute' \
         WHERE job_id = $1",
    )
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Recovery sweep: any job that has been claimed for > 30 minutes without
/// rescheduling is assumed to be from a crashed server. Reset it to run soon.
/// Returns the number of jobs recovered.
pub async fn recover_stuck_jobs(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE cron_jobs \
         SET claimed_at = NULL, \
             next_run_at = NOW() + INTERVAL '1 minute', \
             last_status = 'recovered' \
         WHERE next_run_at IS NULL \
           AND claimed_at IS NOT NULL \
           AND claimed_at < NOW() - INTERVAL '30 minutes' \
           AND enabled = true",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Disable a one-shot job after execution (at-mode, not delete_after_run).
pub async fn disable_job(pool: &PgPool, job_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cron_jobs \
         SET enabled = false, next_run_at = NULL, claimed_at = NULL \
         WHERE job_id = $1",
    )
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 1.7: Verify the crate compiles**

```bash
cargo build --package plexus-server 2>&1 | grep -E "error|warning: unused" | head -30
```

Expected: no errors. There will be warnings about `find_due_jobs` and `update_after_run` no longer existing — those will be fixed in Task 2.

- [ ] **Step 1.8: Commit**

```bash
cd /home/yucheng/Documents/GitHub/Plexus
git add plexus-server/src/db/mod.rs plexus-server/src/db/cron.rs
git commit -m "feat(cron): add claimed_at/last_status columns and atomic DB operations"
```

---

## Task 2: Rewrite `cron.rs` — Claim-Based Poller + Completion Callback

**Files:**
- Modify: `plexus-server/src/cron.rs`

### Context

The current `poll_and_execute` function does three things wrong:
1. Uses `find_due_jobs` (SELECT only — races in multi-node)
2. Computes and writes `next_run_at` immediately after publishing, before the agent runs
3. Deletes/disables jobs immediately, before confirming execution

The new design:
- Calls `recover_stuck_jobs` first (passive sweep, no-ops if nothing is stuck)
- Uses `claim_due_jobs` (atomic UPDATE RETURNING — safe in multi-node)
- Only dispatches to the bus; does NOT touch `next_run_at` after dispatch
- Exposes a new public function `reschedule_after_completion` for agent_loop to call

`compute_next_run` stays in this file because it's used by `reschedule_after_completion`.

---

- [ ] **Step 2.1: Write unit test for `compute_next_run`**

Add this test block at the bottom of `plexus-server/src/cron.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::cron::CronJob;
    use chrono::Utc;

    fn make_job(cron_expr: Option<&str>, every_seconds: Option<i32>) -> CronJob {
        CronJob {
            job_id: "j1".into(),
            user_id: "u1".into(),
            name: "test".into(),
            enabled: true,
            cron_expr: cron_expr.map(String::from),
            every_seconds,
            timezone: "UTC".into(),
            message: "ping".into(),
            channel: "gateway".into(),
            chat_id: "c1".into(),
            delete_after_run: false,
            deliver: true,
            next_run_at: None,
            last_run_at: None,
            run_count: 0,
            created_at: Utc::now(),
            claimed_at: None,
            last_status: None,
        }
    }

    #[test]
    fn every_seconds_schedules_from_now() {
        let job = make_job(None, Some(300));
        let before = Utc::now();
        let next = compute_next_run(&job).expect("should return Some for every_seconds");
        let after = Utc::now();
        // next_run should be 300s from now (within a 2s window)
        assert!(next >= before + chrono::Duration::seconds(298));
        assert!(next <= after + chrono::Duration::seconds(302));
    }

    #[test]
    fn at_mode_returns_none() {
        // No cron_expr, no every_seconds → at-mode, one-shot
        let job = make_job(None, None);
        assert!(compute_next_run(&job).is_none());
    }

    #[test]
    fn cron_expr_returns_future_time() {
        // "0 0 * * * * *" = every minute, at second 0
        let job = make_job(Some("0 0 * * * * *"), None);
        let next = compute_next_run(&job).expect("should return Some for cron_expr");
        assert!(next > Utc::now(), "next run should be in the future");
    }
}
```

- [ ] **Step 2.2: Run tests to confirm they pass (struct already exists)**

```bash
cargo test --package plexus-server cron::tests 2>&1 | tail -15
```

Expected: all three tests pass. If `compute_next_run` has issues with the `cron_expr` format, fix the test expression to match what the cron crate expects (7-field format stored in DB is `"0 0 * * * * *"`).

- [ ] **Step 2.3: Rewrite `cron.rs`**

Replace the entire contents of `plexus-server/src/cron.rs` with:

```rust
//! Cron scheduler: polls DB every 10s for due jobs, injects into message bus.
//!
//! Design mirrors nanobot's "wait-until-done-then-reschedule" pattern:
//! 1. CLAIM  — atomic UPDATE sets next_run_at = NULL (prevents double-firing)
//! 2. DISPATCH — publish InboundEvent to message bus (agent runs async)
//! 3. RESCHEDULE — agent_loop calls reschedule_after_completion() when done
//!
//! Crash recovery: if a server dies after claiming but before rescheduling,
//! recover_stuck_jobs() resets the job after 30 minutes.

use crate::bus::{self, InboundEvent};
use crate::state::AppState;
use plexus_common::consts::CRON_POLL_INTERVAL_SEC;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Spawn the cron poller background task.
pub fn spawn_cron_poller(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(CRON_POLL_INTERVAL_SEC));
        loop {
            interval.tick().await;
            if let Err(e) = poll_and_execute(&state).await {
                warn!("Cron poll error: {e}");
            }
        }
    });
}

async fn poll_and_execute(state: &Arc<AppState>) -> Result<(), String> {
    // Step 1: Recover jobs stuck in claimed state for > 30 minutes (crash recovery).
    match crate::db::cron::recover_stuck_jobs(&state.db).await {
        Ok(n) if n > 0 => warn!("Cron: recovered {n} stuck job(s) from a previous crash"),
        Err(e) => warn!("Cron: stuck job recovery sweep failed: {e}"),
        _ => {}
    }

    // Step 2: Atomically claim all due jobs.
    // The UPDATE ... RETURNING means only one server wins each job even in multi-node.
    let claimed = crate::db::cron::claim_due_jobs(&state.db)
        .await
        .map_err(|e| format!("Claim due jobs: {e}"))?;

    // Step 3: Dispatch each claimed job to the message bus.
    // DO NOT reschedule here — that happens after the agent loop completes (Step 3 in design).
    for job in claimed {
        info!("Cron firing: {} [{}]", job.name, job.job_id);

        let event = InboundEvent {
            session_id: format!("cron:{}", job.job_id),
            user_id: job.user_id.clone(),
            content: job.message.clone(),
            channel: job.channel.clone(),
            chat_id: Some(job.chat_id.clone()),
            sender_id: None,
            media: vec![],
            cron_job_id: Some(job.job_id.clone()),
            identity: None,
            metadata: Default::default(),
        };

        if let Err(e) = bus::publish_inbound(state, event).await {
            // Dispatch failed (bus full, channel broken, etc.) — unclaim immediately
            // so it retries in 1 minute rather than waiting 30 min for stuck recovery.
            error!("Cron dispatch failed for {} [{}]: {e}", job.name, job.job_id);
            let _ = crate::db::cron::unclaim_job(&state.db, &job.job_id).await;
        }
        // Rescheduling intentionally omitted here — see reschedule_after_completion below.
    }

    Ok(())
}

/// Called by agent_loop after a cron event's full ReAct turn completes.
/// Mirrors nanobot's post-execution `_compute_next_run` — the next run time is
/// computed from "now" (after execution finished), not from when the job was claimed.
///
/// `success`: true if handle_event returned Ok(()), false if it returned Err.
pub async fn reschedule_after_completion(
    state: &Arc<AppState>,
    job_id: &str,
    success: bool,
) {
    // Load the job to get its schedule parameters.
    // The job still exists even though next_run_at is NULL (claimed state).
    let job = match crate::db::cron::find_by_id(&state.db, job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            // Job was deleted while it was running (user removed it). Fine — nothing to do.
            info!("Cron: job {job_id} was deleted during execution, skipping reschedule");
            return;
        }
        Err(e) => {
            error!("Cron: failed to load job {job_id} for rescheduling: {e}");
            return;
        }
    };

    if job.delete_after_run {
        // One-shot with delete semantics: remove the job now that it has run.
        let _ = crate::db::cron::delete_job(&state.db, job_id, &job.user_id).await;
        info!("Cron: deleted one-shot job {} [{}]", job.name, job_id);
        return;
    }

    let next_run = compute_next_run(&job);

    if next_run.is_none() {
        // at-mode without delete_after_run: disable (job has fulfilled its single purpose).
        let _ = crate::db::cron::disable_job(&state.db, job_id).await;
        info!("Cron: disabled at-mode job {} [{}]", job.name, job_id);
        return;
    }

    // Recurring job: write the post-execution next_run_at.
    // next_run is computed from Utc::now() inside compute_next_run — this is the key
    // nanobot parity property: the interval starts AFTER execution finishes.
    match crate::db::cron::reschedule_job(&state.db, job_id, next_run, success).await {
        Ok(()) => info!(
            "Cron: rescheduled {} [{}] to {:?} (status={})",
            job.name,
            job_id,
            next_run,
            if success { "ok" } else { "error" }
        ),
        Err(e) => error!("Cron: failed to reschedule job {job_id}: {e}"),
    }
}

fn compute_next_run(
    job: &crate::db::cron::CronJob,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(ref expr) = job.cron_expr {
        crate::server_tools::cron_tool::compute_next_cron_pub(expr, &job.timezone).ok()
    } else if let Some(secs) = job.every_seconds {
        Some(chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::cron::CronJob;
    use chrono::Utc;

    fn make_job(cron_expr: Option<&str>, every_seconds: Option<i32>) -> CronJob {
        CronJob {
            job_id: "j1".into(),
            user_id: "u1".into(),
            name: "test".into(),
            enabled: true,
            cron_expr: cron_expr.map(String::from),
            every_seconds,
            timezone: "UTC".into(),
            message: "ping".into(),
            channel: "gateway".into(),
            chat_id: "c1".into(),
            delete_after_run: false,
            deliver: true,
            next_run_at: None,
            last_run_at: None,
            run_count: 0,
            created_at: Utc::now(),
            claimed_at: None,
            last_status: None,
        }
    }

    #[test]
    fn every_seconds_schedules_from_now() {
        let job = make_job(None, Some(300));
        let before = Utc::now();
        let next = compute_next_run(&job).expect("should return Some for every_seconds");
        let after = Utc::now();
        assert!(next >= before + chrono::Duration::seconds(298));
        assert!(next <= after + chrono::Duration::seconds(302));
    }

    #[test]
    fn at_mode_returns_none() {
        let job = make_job(None, None);
        assert!(compute_next_run(&job).is_none());
    }

    #[test]
    fn cron_expr_returns_future_time() {
        // 7-field format stored in DB by cron_tool (second minute hour dom month dow year)
        let job = make_job(Some("0 0 * * * * *"), None);
        let next = compute_next_run(&job).expect("should return Some for cron_expr");
        assert!(next > Utc::now(), "next run should be in the future");
    }
}
```

- [ ] **Step 2.4: Run unit tests**

```bash
cargo test --package plexus-server cron::tests 2>&1 | tail -15
```

Expected: all three tests pass.

- [ ] **Step 2.5: Build to check for compile errors**

```bash
cargo build --package plexus-server 2>&1 | grep "^error" | head -20
```

Expected: no errors. The only errors at this point would be in `agent_loop.rs` if anything there references the removed `find_due_jobs`/`update_after_run` — it doesn't, so this should be clean.

- [ ] **Step 2.6: Commit**

```bash
git add plexus-server/src/cron.rs
git commit -m "feat(cron): rewrite poller with atomic claim and post-execution reschedule callback"
```

---

## Task 3: Agent Loop Feedback — Call Reschedule After Completion

**Files:**
- Modify: `plexus-server/src/agent_loop.rs`

### Context

`run_session` in `agent_loop.rs` processes events in a loop:

```rust
while let Some(event) = inbox.recv().await {
    // ...
    if let Err(e) = handle_event(&state, &session_id, &user_id, event).await {
        error!("Session {session_id} error: {e}");
    }
}
```

`handle_event` takes `event` by value (moves it), so we must extract `cron_job_id` before the call. After `handle_event` returns — which means the full ReAct turn (all tool calls, all LLM iterations) has finished — we call `cron::reschedule_after_completion`. This is the exact equivalent of nanobot's `await self.on_job(job)` returning.

`handle_event` returns `Result<(), String>`. We use `is_ok()` to pass the success flag.

---

- [ ] **Step 3.1: Modify the event processing loop in `run_session`**

In `plexus-server/src/agent_loop.rs`, find this block inside `run_session`:

```rust
        if let Err(e) = handle_event(&state, &session_id, &user_id, event).await {
            error!("Session {session_id} error: {e}");
        }
```

Replace it with:

```rust
        // Extract cron_job_id before event is moved into handle_event.
        let cron_job_id = event.cron_job_id.clone();

        let result = handle_event(&state, &session_id, &user_id, event).await;

        // If this was a cron event, notify the scheduler that the full ReAct
        // turn has finished. This is the nanobot-parity "wait until done" step:
        // next_run_at is computed from now (after execution), not from dispatch time.
        if let Some(ref job_id) = cron_job_id {
            crate::cron::reschedule_after_completion(&state, job_id, result.is_ok()).await;
        }

        if let Err(e) = result {
            error!("Session {session_id} error: {e}");
        }
```

- [ ] **Step 3.2: Build to confirm no compile errors**

```bash
cargo build --package plexus-server 2>&1 | grep "^error" | head -20
```

Expected: clean build, no errors.

- [ ] **Step 3.3: Run all unit tests**

```bash
cargo test --package plexus-server 2>&1 | tail -20
```

Expected: all existing tests pass plus the new cron tests.

- [ ] **Step 3.4: Run clippy**

```bash
cargo clippy --package plexus-server 2>&1 | grep "^error" | head -20
```

Expected: no errors. Fix any warnings about unused imports from the removed functions.

- [ ] **Step 3.5: Commit**

```bash
git add plexus-server/src/agent_loop.rs
git commit -m "feat(cron): notify scheduler after agent loop completes cron event turn"
```

---

## Task 4: Full Build Verification

**Files:** None (verification only)

- [ ] **Step 4.1: Full workspace build**

```bash
cd /home/yucheng/Documents/GitHub/Plexus
cargo build 2>&1 | grep "^error" | head -20
```

Expected: zero errors across all crates.

- [ ] **Step 4.2: Full workspace clippy**

```bash
cargo clippy 2>&1 | grep "^error" | head -20
```

Expected: zero errors.

- [ ] **Step 4.3: Full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4.4: Manual behavior checklist**

Start Plexus and create a test cron job via the cron tool:

```
every_seconds: 30, message: "test cron ping", channel: "gateway"
```

Verify in server logs (add `RUST_LOG=info`):

| What to check | Expected log |
|---|---|
| Job fires at T=0 | `Cron firing: test [<job_id>]` |
| Job is claimed (not re-queued) | No second `Cron firing` until agent finishes |
| Agent completes turn | `Cron: rescheduled test [<job_id>] to ... (status=ok)` |
| 30s after agent finishes | Second `Cron firing` (not 30s after original dispatch) |
| Server restart mid-execution | After restart, within 30 min: `Cron: recovered 1 stuck job(s)` |

- [ ] **Step 4.5: Final commit**

```bash
git add -p  # review any remaining changes
git commit -m "chore: verify cron nanobot-parity implementation complete"
```

---

## Summary of Changes

### What was broken
- `next_run_at` was computed and written **immediately at dispatch time** — before the agent started
- A SELECT-based `find_due_jobs` allowed multiple servers (or a fast poller) to double-fire jobs
- Server crash between dispatch and DB update permanently lost the execution record

### What is fixed
| Issue | Fix |
|---|---|
| Overlapping runs | `claim_due_jobs` sets `next_run_at = NULL`; poller skips jobs with NULL `next_run_at` |
| Double-firing (multi-node) | `UPDATE ... RETURNING` is atomic — PostgreSQL gives each row to exactly one winner |
| Drifting schedules | `reschedule_after_completion` computes next run from `Utc::now()` after agent finishes |
| Lost executions on crash | `claimed_at` timestamp; `recover_stuck_jobs` resets after 30 min |
| No error tracking | `last_status = "ok" / "error" / "recovered"` written on every state change |
