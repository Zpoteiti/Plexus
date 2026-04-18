//! Dream subsystem: periodic memory consolidation + skill discovery.
//!
//! Wired into the cron poller (D-7): when a kind='system' AND name='dream'
//! job fires, cron.rs dispatches to handle_dream_fire instead of publishing
//! a regular Cron InboundEvent.
//!
//! The handler does a cheap idle check (DB-only, no LLM cost), and on
//! positive activity runs Phase 1 — a standalone LLM call with the
//! dream_phase1 prompt that emits structured directives. Non-empty
//! directives are published as an InboundEvent { kind: Dream } which
//! the agent loop (D-8) routes to PromptMode::Dream + ToolAllowlist::
//! Only(DREAM_PHASE2_ALLOWLIST).

use crate::bus::{EventKind, InboundEvent};
use crate::state::AppState;
use std::sync::Arc;
use tracing::{info, warn};

/// Maximum number of recent messages Phase 1 reads into its analysis window.
/// Plan D §3.4 fixes this at 200 to bound LLM input size for chatty users.
const PHASE1_MESSAGE_CAP: i64 = 200;

/// Invoked by the cron poller when a kind='system' name='dream' job fires.
///
/// Flow:
/// 1. Global kill switch (system_config.dream_enabled = false → skip).
/// 2. Cheap idle check (DB query, no LLM): skip if no activity since last dream.
/// 3. Advance users.last_dream_at BEFORE running phases (prevents refire
///    during execution; failure modes don't block future dreams).
/// 4. Phase 1: standalone LLM call with dream_phase1 prompt, history slice,
///    MEMORY.md, SOUL.md, and skills index. Output is free-form text of
///    directives.
/// 5. If Phase 1 returns empty or `[NO-OP]`, stop and reschedule.
/// 6. Otherwise publish an InboundEvent { kind: Dream, content: directives }
///    — the agent loop picks it up as a Phase 2 run.
///
/// On any error path (DB failure, LLM unreachable, etc.), logs a warn!
/// and falls back to rescheduling so the cron poller retries next cycle.
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
        (None, _) => false,           // no activity ever
        (Some(_), None) => true,      // first dream
        (Some(a), Some(d)) => a > d,  // new activity since last dream
    };
    if !should_dream {
        info!(user_id = %job.user_id, "dream: no new activity, skipping");
        reschedule(state, &job.job_id, true).await;
        return Ok(());
    }

    // 3. Advance last_dream_at BEFORE phases.
    let now = chrono::Utc::now();
    if let Err(e) = crate::db::users::update_last_dream_at(&state.db, &job.user_id, now).await {
        warn!(error = %e, user_id = %job.user_id, "dream: failed to advance last_dream_at; skipping");
        reschedule(state, &job.job_id, false).await;
        return Err(format!("update_last_dream_at: {e}"));
    }

    // 4. Phase 1: standalone LLM call.
    let directives = run_phase1(state, &job.user_id, last_dream).await;

    // 5. NO-OP short-circuit.
    let trimmed = directives.trim();
    if trimmed.is_empty() || trimmed == "[NO-OP]" {
        info!(user_id = %job.user_id, "dream: phase 1 emitted NO-OP, skipping phase 2");
        reschedule(state, &job.job_id, true).await;
        return Ok(());
    }

    // 6. Publish Phase 2 event. Agent loop's reschedule_after_completion
    //    fires when the turn ends.
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

/// Reads system_config.dream_enabled (default: true).
async fn is_dream_enabled(pool: &sqlx::PgPool) -> bool {
    match crate::db::system_config::get(pool, "dream_enabled").await {
        Ok(Some(v)) => v.trim() != "false",
        Ok(None) => true, // A-20 seeds "true"; if unset we still default-on
        Err(e) => {
            warn!(error = %e, "dream: dream_enabled lookup failed, defaulting to enabled");
            true
        }
    }
}

/// Thin wrapper around `cron::reschedule_after_completion` for the dream
/// paths that short-circuit before publishing. On the publish path we
/// rely on agent_loop's post-turn hook instead.
async fn reschedule(state: &Arc<AppState>, job_id: &str, success: bool) {
    crate::cron::reschedule_after_completion(state, job_id, success).await;
}

/// Phase 1 standalone LLM call. Reads inputs, builds a single-turn chat,
/// calls the LLM with no tools. Returns directives as a raw text blob.
/// On any error, returns an empty string (caller treats as NO-OP).
async fn run_phase1(
    state: &Arc<AppState>,
    user_id: &str,
    last_dream: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let user_root = ws_root.join(user_id);

    let memory = tokio::fs::read_to_string(user_root.join("MEMORY.md"))
        .await
        .unwrap_or_default();
    let soul = tokio::fs::read_to_string(user_root.join("SOUL.md"))
        .await
        .unwrap_or_default();

    // Skills index: name + description only (no bodies — Phase 1 decides if
    // a skill should exist, Phase 2 reads the body if needed).
    let bundle = state.skills_cache.get_or_load(user_id, ws_root).await;
    let skills_index = if bundle.is_empty() {
        "(no skills yet)".to_string()
    } else {
        bundle
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Messages since last dream. For first-ever dream, fall back to epoch.
    let since = last_dream.unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH);

    let messages = match crate::db::messages::get_messages_since(
        &state.db,
        user_id,
        since,
        PHASE1_MESSAGE_CAP,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, user_id, "dream phase 1: failed to fetch messages");
            return String::new();
        }
    };

    let activity = if messages.is_empty() {
        "(no messages in window)".to_string()
    } else {
        messages
            .iter()
            .map(|m| format!("[{}] {}: {}", m.created_at, m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let system_prompt = state.dream_phase1_prompt.read().await.clone();
    let user_body = format!(
        "## Current MEMORY.md\n\n{memory}\n\n\
         ## Current SOUL.md\n\n{soul}\n\n\
         ## Skills index\n\n{skills_index}\n\n\
         ## Recent activity\n\n{activity}"
    );

    // LLM config may be missing (fresh install before admin sets it). Silent
    // fallthrough to empty directives.
    let llm_config = match state.llm_config.read().await.clone() {
        Some(c) => c,
        None => {
            warn!(user_id, "dream phase 1: no LLM config, skipping");
            return String::new();
        }
    };

    let chat_messages = vec![
        crate::providers::openai::ChatMessage::system(system_prompt),
        crate::providers::openai::ChatMessage::user(user_body),
    ];

    match crate::providers::openai::call_llm(
        &state.http_client,
        &llm_config,
        chat_messages,
        None, // no tools — Phase 1 emits raw text
        None, // no tool_choice
    )
    .await
    {
        Ok(crate::providers::openai::LlmResponse::Text { content, .. }) => content,
        Ok(_) => {
            warn!(user_id, "dream phase 1: LLM returned unexpected response shape");
            String::new()
        }
        Err(e) => {
            warn!(error = %e, user_id, "dream phase 1: LLM call failed");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    /// Sanity test on the trimming logic used to detect NO-OP responses.
    #[test]
    fn noop_detection_matrix() {
        let cases: &[(&str, bool)] = &[
            ("", true),
            ("   ", true),
            ("\n[NO-OP]\n", true),
            ("[NO-OP]", true),
            ("[MEMORY-ADD] ## User Facts\n- foo", false),
            ("[NO-OP] with trailing junk", false), // exact match after trim
            ("  hello world  ", false),
        ];
        for (input, expected_noop) in cases {
            let trimmed = input.trim();
            let is_noop = trimmed.is_empty() || trimmed == "[NO-OP]";
            assert_eq!(
                is_noop, *expected_noop,
                "input={input:?} — got is_noop={is_noop}, expected {expected_noop}"
            );
        }
    }
}
