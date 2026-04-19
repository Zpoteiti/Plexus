//! Cron scheduler: polls DB every 10s for due jobs, injects into message bus.
//!
//! Design mirrors nanobot's "wait-until-done-then-reschedule" pattern:
//! 1. CLAIM  — atomic UPDATE sets next_run_at = NULL (prevents double-firing)
//! 2. DISPATCH — publish InboundEvent to message bus (agent runs async)
//! 3. RESCHEDULE — agent_loop calls reschedule_after_completion() when done
//!
//! Crash recovery: if a server dies after claiming but before rescheduling,
//! recover_stuck_jobs() resets the job after 30 minutes.

use crate::bus::{self, EventKind, InboundEvent};
use crate::consts::CRON_POLL_INTERVAL_SEC;
use crate::state::AppState;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Spawn the cron poller background task.
pub fn spawn_cron_poller(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(CRON_POLL_INTERVAL_SEC));
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    info!("cron poller shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = poll_and_execute(&state).await {
                        warn!("Cron poll error: {e}");
                    }
                }
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
    // DO NOT reschedule here — that happens after the agent loop completes.
    for job in claimed {
        info!(
            "Cron firing: {} [{}] kind={}",
            job.name, job.job_id, job.kind
        );

        // Dream system-cron: dispatch to the dream handler on a new task.
        // handle_dream_fire owns the decision to skip (idle check, NO-OP
        // directives) vs. publish. It also owns rescheduling for the skip
        // paths; the publish path defers to agent_loop's post-turn hook.
        // Spawned so the poller doesn't block its own tick loop during
        // Phase 1's LLM call.
        if job.kind == crate::db::cron::SYSTEM_KIND && job.name == "dream" {
            let state = Arc::clone(state);
            let job = job.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::dream::handle_dream_fire(&state, &job).await {
                    warn!(job_id = %job.job_id, "dream handler error: {e}");
                    // handle_dream_fire does NOT reschedule on all Err paths
                    // (e.g. publish_inbound failure, idle-check DB errors).
                    // The wrapper is the sole rescheduler for those cases.
                    crate::cron::reschedule_after_completion(&state, &job.job_id, false).await;
                }
            });
            continue;
        }

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

        if let Err(e) = bus::publish_inbound(state, event).await {
            // Dispatch failed — unclaim immediately so it retries in 1 minute
            // rather than waiting 30 min for the stuck recovery sweep.
            error!(
                "Cron dispatch failed for {} [{}]: {e}",
                job.name, job.job_id
            );
            if let Err(ue) = crate::db::cron::unclaim_job(&state.db, &job.job_id).await {
                warn!(
                    "Cron: failed to unclaim job {} after dispatch failure: {ue}",
                    job.job_id
                );
            }
        }
        // Rescheduling intentionally omitted — see reschedule_after_completion below.
    }

    Ok(())
}

/// Called by agent_loop after a cron event's full ReAct turn completes.
/// Mirrors nanobot's post-execution `_compute_next_run` — the next run time is
/// computed from "now" (after execution finished), not from when the job was claimed.
///
/// `success`: true if handle_event returned Ok(()), false if it returned Err.
pub async fn reschedule_after_completion(state: &Arc<AppState>, job_id: &str, success: bool) {
    // Load the job to get its schedule parameters.
    // The job still exists even though next_run_at is NULL (claimed state).
    let job = match crate::db::cron::find_by_id(&state.db, job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            // Job was deleted while running (user removed it). Nothing to do.
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
        if let Err(e) = crate::db::cron::delete_job(&state.db, job_id, &job.user_id).await {
            error!("Cron: failed to delete one-shot job {job_id}: {e}");
        } else {
            info!("Cron: deleted one-shot job {} [{}]", job.name, job_id);
        }
        return;
    }

    let next_run = compute_next_run(&job);

    if next_run.is_none() {
        // at-mode without delete_after_run: disable (job has fulfilled its single purpose).
        if let Err(e) = crate::db::cron::disable_job(&state.db, job_id).await {
            error!("Cron: failed to disable at-mode job {job_id}: {e}");
        } else {
            info!("Cron: disabled at-mode job {} [{}]", job.name, job_id);
        }
        return;
    }

    // Recurring job: write the post-execution next_run_at.
    // next_run is computed from Utc::now() inside compute_next_run — the key
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

fn compute_next_run(job: &crate::db::cron::CronJob) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(ref expr) = job.cron_expr {
        crate::server_tools::cron_tool::compute_next_cron_pub(expr, &job.timezone).ok()
    } else {
        job.every_seconds
            .map(|secs| chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
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
            kind: "user".into(),
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
