use crate::{
    chat::prompt,
    db::{messages, pending_messages, sessions, system_config},
    openai::{ChatCompletionRequest, ChatMessage},
};
use plexus_common::{ChatRole, ContentBlock};
use uuid::Uuid;

pub async fn spawn_pending_workers(
    state: crate::app::AppState,
) -> Result<(), crate::error::ApiError> {
    let pending = pending_messages::pending_sessions(state.pool())
        .await
        .map_err(crate::error::ApiError::from_sqlx)?;
    for row in pending {
        let effort = row.reasoning_effort.parse().map_err(|_| {
            crate::error::ApiError::invalid_args("stored pending reasoning_effort was malformed")
        })?;
        if state.chat().enqueue_turn(row.session_id, effort).await {
            spawn_response_worker(state.clone(), row.session_id);
        }
    }
    Ok(())
}

pub fn spawn_response_worker(state: crate::app::AppState, session_id: Uuid) {
    tokio::spawn(async move {
        loop {
            let result = run_worker_loop(state.clone(), session_id).await;
            if let Err(err) = result {
                let content = vec![ContentBlock::text(synthetic_error_text(&err.message))];
                if let Ok(message) =
                    messages::insert_message(state.pool(), session_id, "assistant", content).await
                {
                    state.chat().broker().broadcast(message).await;
                }
            }
            if !state.chat().finish_or_continue_worker(session_id).await {
                break;
            }
        }
    });
}

async fn run_worker_loop(
    state: crate::app::AppState,
    session_id: Uuid,
) -> Result<(), crate::error::ApiError> {
    let mut last_answered_user_id = None;
    loop {
        state.chat().clear_observed_wake(session_id).await;
        drain_pending_at_safe_boundary(state.clone(), session_id).await?;
        let Some(latest) = messages::latest_user_message(state.pool(), session_id)
            .await
            .map_err(crate::error::ApiError::from_sqlx)?
        else {
            return Ok(());
        };
        if Some(latest.id) == last_answered_user_id {
            return Ok(());
        }

        last_answered_user_id = Some(respond_once(state.clone(), session_id).await?);
    }
}

async fn drain_pending_at_safe_boundary(
    state: crate::app::AppState,
    session_id: Uuid,
) -> Result<(), crate::error::ApiError> {
    let drained = pending_messages::drain_for_session(state.pool(), session_id)
        .await
        .map_err(crate::error::ApiError::from_sqlx)?;
    for (message, effort) in drained {
        state
            .chat()
            .update_reasoning_effort(session_id, effort)
            .await;
        state.chat().broker().broadcast(message).await;
    }
    Ok(())
}

async fn respond_once(
    state: crate::app::AppState,
    session_id: Uuid,
) -> Result<Uuid, crate::error::ApiError> {
    let session = sessions::find_by_id(state.pool(), session_id)
        .await
        .map_err(crate::error::ApiError::from_sqlx)?
        .ok_or_else(|| crate::error::ApiError::invalid_args("session disappeared"))?;
    let user = crate::db::users::find_by_id(state.pool(), session.user_id)
        .await
        .map_err(crate::error::ApiError::from_sqlx)?
        .ok_or_else(|| crate::error::ApiError::invalid_args("session user disappeared"))?;
    let cfg = system_config::current_llm_config(state.pool()).await?;
    let reasoning_effort = state
        .chat()
        .reasoning_effort(session_id)
        .await
        .ok_or_else(|| {
            crate::error::ApiError::invalid_args(
                "reasoning_effort is required for pending chat turn",
            )
        })?;
    let system_prompt =
        prompt::build_system_prompt(&state.config().workspace_root, &user, &session).await?;
    let history = messages::history_chronological(state.pool(), session_id)
        .await
        .map_err(crate::error::ApiError::from_sqlx)?;
    let (chat_messages, last_user_id) = build_chat_messages(system_prompt, history)?;
    let last_user_id = last_user_id
        .ok_or_else(|| crate::error::ApiError::invalid_args("no user message in chat history"))?;

    let response = state
        .openai()
        .chat_completion(
            &cfg,
            ChatCompletionRequest {
                messages: chat_messages,
                max_tokens: None,
                temperature: None,
                reasoning_effort,
            },
        )
        .await;

    let (assistant_text, reasoning_content) = match response {
        Ok(response) => (response.content, response.reasoning_content),
        Err(err) => (synthetic_error_text(&err.message), None),
    };
    let message = messages::insert_message_with_reasoning(
        state.pool(),
        session_id,
        "assistant",
        vec![ContentBlock::text(assistant_text)],
        reasoning_content,
    )
    .await
    .map_err(crate::error::ApiError::from_sqlx)?;
    state.chat().broker().broadcast(message).await;
    Ok(last_user_id)
}

fn build_chat_messages(
    system_prompt: String,
    history: Vec<messages::Message>,
) -> Result<(Vec<ChatMessage>, Option<Uuid>), crate::error::ApiError> {
    let mut last_user_id = None;
    let mut chat_messages = vec![ChatMessage {
        role: ChatRole::System,
        content: vec![ContentBlock::text(system_prompt)],
        reasoning_content: None,
    }];
    for row in history {
        let role = match row.role.as_str() {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => continue,
        };
        if role == ChatRole::User {
            last_user_id = Some(row.id);
        }
        let content = serde_json::from_value(row.content).map_err(|_| {
            crate::error::ApiError::invalid_args("stored message content was malformed")
        })?;
        chat_messages.push(ChatMessage {
            role,
            content,
            reasoning_content: row.reasoning_content,
        });
    }
    Ok((chat_messages, last_user_id))
}

fn synthetic_error_text(message: &str) -> String {
    let safe = message
        .replace("Bearer ", "")
        .replace("plexus-mock-key", "[redacted]");
    format!("[Plexus could not complete the LLM request: {safe}. Try again later.]")
}

#[cfg(test)]
mod tests {
    use super::build_chat_messages;
    use crate::db::messages::Message;
    use plexus_common::{ChatRole, ContentBlock};
    use serde_json::json;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn message(id: Uuid, role: &str, text: &str) -> Message {
        Message {
            id,
            session_id: Uuid::now_v7(),
            role: role.to_string(),
            content: json!([{"type": "text", "text": text}]),
            reasoning_content: None,
            is_compaction_summary: false,
            created_at: OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn chat_messages_report_last_user_id_from_snapshot() {
        let first_user_id = Uuid::now_v7();
        let second_user_id = Uuid::now_v7();
        let history = vec![
            message(first_user_id, "user", "one"),
            message(Uuid::now_v7(), "assistant", "answer"),
            message(second_user_id, "user", "two"),
        ];

        let (chat_messages, last_user_id) =
            build_chat_messages("system prompt".to_string(), history).unwrap();

        assert_eq!(last_user_id, Some(second_user_id));
        assert_eq!(chat_messages[0].role, ChatRole::System);
        assert_eq!(chat_messages[1].role, ChatRole::User);
        assert_eq!(chat_messages[3].role, ChatRole::User);
        assert_eq!(chat_messages[3].content, vec![ContentBlock::text("two")]);
    }
}
