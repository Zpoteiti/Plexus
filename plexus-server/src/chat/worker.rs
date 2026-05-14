use crate::{
    chat::prompt,
    db::{messages, sessions, system_config},
    openai::{ChatCompletionRequest, ChatMessage},
};
use plexus_common::{ChatRole, ContentBlock};
use uuid::Uuid;

pub fn spawn_response_worker(state: crate::app::AppState, session_id: Uuid) {
    tokio::spawn(async move {
        if !state.chat().try_start_worker(session_id).await {
            return;
        }
        let result = run_worker_loop(state.clone(), session_id).await;
        state.chat().finish_worker(session_id).await;
        if let Err(err) = result {
            let content = vec![ContentBlock::text(synthetic_error_text(&err.message))];
            if let Ok(message) =
                messages::insert_message(state.pool(), session_id, "assistant", content).await
            {
                state.chat().broker().broadcast(message).await;
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
        let Some(latest) = messages::latest_user_message(state.pool(), session_id)
            .await
            .map_err(crate::error::ApiError::from_sqlx)?
        else {
            return Ok(());
        };
        if Some(latest.id) == last_answered_user_id {
            return Ok(());
        }

        respond_once(state.clone(), session_id).await?;
        last_answered_user_id = Some(latest.id);
    }
}

async fn respond_once(
    state: crate::app::AppState,
    session_id: Uuid,
) -> Result<(), crate::error::ApiError> {
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
        let content = serde_json::from_value(row.content).map_err(|_| {
            crate::error::ApiError::invalid_args("stored message content was malformed")
        })?;
        chat_messages.push(ChatMessage {
            role,
            content,
            reasoning_content: row.reasoning_content,
        });
    }

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
    Ok(())
}

fn synthetic_error_text(message: &str) -> String {
    let safe = message
        .replace("Bearer ", "")
        .replace("plexus-mock-key", "[redacted]");
    format!("[Plexus could not complete the LLM request: {safe}. Try again later.]")
}
