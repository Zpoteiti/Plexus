use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Message {
    pub message_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub compressed: bool,
    pub created_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    message_id: &str,
    session_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO messages (message_id, session_id, role, content, tool_call_id, tool_name, tool_arguments)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(message_id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
    .bind(tool_name)
    .bind(tool_arguments)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_uncompressed(
    pool: &PgPool,
    session_id: &str,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE session_id = $1 AND compressed = FALSE ORDER BY created_at ASC LIMIT $2",
    )
    .bind(session_id)
    .bind(crate::consts::MAX_UNCOMPRESSED_MESSAGES)
    .fetch_all(pool)
    .await
}

pub async fn list_paginated(
    pool: &PgPool,
    session_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE session_id = $1 ORDER BY created_at ASC LIMIT $2 OFFSET $3",
    )
    .bind(session_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn mark_compressed(pool: &PgPool, message_ids: &[String]) -> Result<(), sqlx::Error> {
    if message_ids.is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE messages SET compressed = TRUE WHERE message_id = ANY($1)")
        .bind(message_ids)
        .execute(pool)
        .await?;
    Ok(())
}

/// Most recent message timestamp across all non-autonomous sessions for a user.
/// Excludes 'dream:*' and 'heartbeat:*' sessions (these are autonomous pipelines;
/// including them would create a feedback loop where dream retriggers itself).
/// Returns None if the user has never sent / received a message.
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
/// Used by Plan D's Phase 1 to feed the analysis LLM the activity window.
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
