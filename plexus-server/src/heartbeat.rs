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
#[allow(dead_code)] // consumed in E-8 tick_once body
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
