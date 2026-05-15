use plexus_common::ContentBlock;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: String,
    pub content: Value,
    pub reasoning_content: Option<String>,
    pub is_compaction_summary: bool,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LatestUserMessage {
    pub id: Uuid,
    pub created_at: OffsetDateTime,
}

pub async fn insert_message(
    pool: &PgPool,
    session_id: Uuid,
    role: &str,
    content: Vec<ContentBlock>,
) -> Result<Message, sqlx::Error> {
    insert_message_with_reasoning(pool, session_id, role, content, None).await
}

pub async fn insert_message_with_reasoning(
    pool: &PgPool,
    session_id: Uuid,
    role: &str,
    content: Vec<ContentBlock>,
    reasoning_content: Option<String>,
) -> Result<Message, sqlx::Error> {
    let content = serde_json::to_value(content).expect("content blocks serialize");
    sqlx::query_as::<_, Message>(
        r#"
        INSERT INTO messages (session_id, role, content, reasoning_content)
        VALUES ($1, $2, $3, $4)
        RETURNING id, session_id, role, content, reasoning_content, is_compaction_summary, created_at
        "#,
    )
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(reasoning_content)
    .fetch_one(pool)
    .await
}

pub async fn list_before(
    pool: &PgPool,
    session_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Message>, sqlx::Error> {
    if let Some(before) = before {
        sqlx::query_as::<_, Message>(
            r#"
            SELECT m.id, m.session_id, m.role, m.content, m.reasoning_content, m.is_compaction_summary, m.created_at
            FROM messages m
            JOIN messages anchor ON anchor.id = $2 AND anchor.session_id = $1
            WHERE m.session_id = $1
              AND (m.created_at < anchor.created_at
                   OR (m.created_at = anchor.created_at AND m.id < anchor.id))
            ORDER BY m.created_at DESC, m.id DESC
            LIMIT $3
            "#,
        )
        .bind(session_id)
        .bind(before)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, Message>(
            r#"
            SELECT id, session_id, role, content, reasoning_content, is_compaction_summary, created_at
            FROM messages
            WHERE session_id = $1
            ORDER BY created_at DESC, id DESC
            LIMIT $2
            "#,
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

pub async fn replay_recent(
    pool: &PgPool,
    session_id: Uuid,
    limit: i64,
) -> Result<Vec<Message>, sqlx::Error> {
    let mut rows = sqlx::query_as::<_, Message>(
        r#"
        SELECT id, session_id, role, content, reasoning_content, is_compaction_summary, created_at
        FROM messages
        WHERE session_id = $1
        ORDER BY created_at DESC, id DESC
        LIMIT $2
        "#,
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.reverse();
    Ok(rows)
}

pub async fn replay_after(
    pool: &PgPool,
    session_id: Uuid,
    after: Uuid,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        r#"
        SELECT m.id, m.session_id, m.role, m.content, m.reasoning_content, m.is_compaction_summary, m.created_at
        FROM messages m
        JOIN messages anchor ON anchor.id = $2 AND anchor.session_id = $1
        WHERE m.session_id = $1
          AND (m.created_at > anchor.created_at
               OR (m.created_at = anchor.created_at AND m.id > anchor.id))
        ORDER BY m.created_at ASC, m.id ASC
        "#,
    )
    .bind(session_id)
    .bind(after)
    .fetch_all(pool)
    .await
}

pub async fn latest_user_message(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<LatestUserMessage>, sqlx::Error> {
    sqlx::query_as::<_, LatestUserMessage>(
        r#"
        SELECT id, created_at
        FROM messages
        WHERE session_id = $1 AND role = 'user'
        ORDER BY created_at DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
}

pub async fn sessions_with_latest_user_message(pool: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT session_id
        FROM (
            SELECT DISTINCT ON (session_id) session_id, role, created_at, id
            FROM messages
            ORDER BY session_id, created_at DESC, id DESC
        ) latest
        WHERE role = 'user'
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(session_id,)| session_id).collect())
}

pub async fn history_chronological(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        r#"
        SELECT id, session_id, role, content, reasoning_content, is_compaction_summary, created_at
        FROM messages
        WHERE session_id = $1
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
}
