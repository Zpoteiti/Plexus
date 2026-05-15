use crate::db::{messages::Message, sessions::Session};
use plexus_common::{ContentBlock, ReasoningEffort};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingMessage {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub session_key: String,
    pub content: Value,
    pub reasoning_effort: String,
    pub received_at: OffsetDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingSession {
    pub session_id: Uuid,
    pub reasoning_effort: String,
}

pub async fn insert_pending(
    pool: &PgPool,
    session: &Session,
    content: Vec<ContentBlock>,
    reasoning_effort: ReasoningEffort,
) -> Result<PendingMessage, sqlx::Error> {
    let id = Uuid::now_v7();
    let content = serde_json::to_value(content).expect("content blocks serialize");
    sqlx::query_as::<_, PendingMessage>(
        r#"
        INSERT INTO pending_messages (id, session_id, user_id, session_key, content, reasoning_effort)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, session_id, user_id, session_key, content, reasoning_effort, received_at
        "#,
    )
    .bind(id)
    .bind(session.id)
    .bind(session.user_id)
    .bind(&session.session_key)
    .bind(content)
    .bind(reasoning_effort.as_str())
    .fetch_one(pool)
    .await
}

pub async fn drain_for_session(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Vec<(Message, ReasoningEffort)>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let pending = pending_for_update(&mut tx, session_id).await?;
    let mut drained = Vec::with_capacity(pending.len());

    for row in &pending {
        let message = sqlx::query_as::<_, Message>(
            r#"
            INSERT INTO messages (id, session_id, role, content)
            VALUES ($1, $2, 'user', $3)
            RETURNING id, session_id, role, content, reasoning_content, is_compaction_summary, created_at
            "#,
        )
        .bind(row.id)
        .bind(row.session_id)
        .bind(row.content.clone())
        .fetch_one(&mut *tx)
        .await?;
        let effort = row
            .reasoning_effort
            .parse::<ReasoningEffort>()
            .expect("pending reasoning_effort constrained by schema");
        drained.push((message, effort));
    }

    if !pending.is_empty() {
        let ids: Vec<Uuid> = pending.iter().map(|row| row.id).collect();
        sqlx::query("DELETE FROM pending_messages WHERE id = ANY($1)")
            .bind(ids)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(drained)
}

pub async fn pending_sessions(pool: &PgPool) -> Result<Vec<PendingSession>, sqlx::Error> {
    sqlx::query_as::<_, PendingSession>(
        r#"
        SELECT DISTINCT ON (session_id) session_id, reasoning_effort
        FROM pending_messages
        ORDER BY session_id, received_at DESC, id DESC
        "#,
    )
    .fetch_all(pool)
    .await
}

async fn pending_for_update(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
) -> Result<Vec<PendingMessage>, sqlx::Error> {
    sqlx::query_as::<_, PendingMessage>(
        r#"
        SELECT id, session_id, user_id, session_key, content, reasoning_effort, received_at
        FROM pending_messages
        WHERE session_id = $1
        ORDER BY received_at ASC, id ASC
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await
}
