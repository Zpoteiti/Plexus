//! Shared post-run evaluator for autonomous agent outputs (cron, heartbeat).
//!
//! Given an agent's final message and a purpose label, returns whether the
//! user should be pinged. The LLM sees the user's current local time so it
//! can reason about "is this a good time to interrupt?" — the 4 AM guard.
//!
//! Default silence on any error — silence is the safe failure mode for
//! notification decisions.

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
        Ok(_) => {
            warn!(user_id, purpose, "evaluator: LLM did not return a tool call, defaulting to silence");
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
        assert!(
            result.reason.contains("no LLM config") || result.reason.contains("LLM"),
            "got reason: {}",
            result.reason
        );
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
