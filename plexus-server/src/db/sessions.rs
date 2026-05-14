use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub const WEB_CHANNEL: &str = "web";
pub const DEFAULT_TITLE: &str = "New chat";
pub const MAX_TITLE_CHARS: usize = 120;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub session_key: String,
    pub channel: String,
    pub chat_id: String,
    pub title: String,
    pub last_inbound_at: Option<OffsetDateTime>,
    pub cancel_requested: bool,
    pub created_at: OffsetDateTime,
}

pub fn normalize_create_title(input: Option<&str>) -> Result<String, String> {
    let title = input.unwrap_or("").trim();
    let title = if title.is_empty() {
        DEFAULT_TITLE
    } else {
        title
    };
    validate_title(title)?;
    Ok(title.to_string())
}

pub fn normalize_rename_title(input: &str) -> Result<String, String> {
    let title = input.trim();
    if title.is_empty() {
        return Err("title must not be empty".to_string());
    }
    validate_title(title)?;
    Ok(title.to_string())
}

fn validate_title(title: &str) -> Result<(), String> {
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "title must be at most {MAX_TITLE_CHARS} characters"
        ));
    }
    Ok(())
}

pub async fn create_web_session(
    pool: &PgPool,
    user_id: Uuid,
    title: &str,
) -> Result<Session, sqlx::Error> {
    let id = Uuid::now_v7();
    let chat_id = id.to_string();
    let session_key = format!("{WEB_CHANNEL}:{chat_id}");
    sqlx::query_as::<_, Session>(
        r#"
        INSERT INTO sessions (id, user_id, session_key, channel, chat_id, title)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, user_id, session_key, channel, chat_id, title,
                  last_inbound_at, cancel_requested, created_at
        "#,
    )
    .bind(id)
    .bind(user_id)
    .bind(session_key)
    .bind(WEB_CHANNEL)
    .bind(chat_id)
    .bind(title)
    .fetch_one(pool)
    .await
}

pub async fn list_for_user(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(
        r#"
        SELECT id, user_id, session_key, channel, chat_id, title,
               last_inbound_at, cancel_requested, created_at
        FROM sessions
        WHERE user_id = $1
        ORDER BY last_inbound_at DESC NULLS LAST, created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn find_owned(
    pool: &PgPool,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(
        r#"
        SELECT id, user_id, session_key, channel, chat_id, title,
               last_inbound_at, cancel_requested, created_at
        FROM sessions
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_id(pool: &PgPool, session_id: Uuid) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(
        r#"
        SELECT id, user_id, session_key, channel, chat_id, title,
               last_inbound_at, cancel_requested, created_at
        FROM sessions
        WHERE id = $1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
}

pub async fn rename_owned(
    pool: &PgPool,
    user_id: Uuid,
    session_id: Uuid,
    title: &str,
) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(
        r#"
        UPDATE sessions
        SET title = $3
        WHERE id = $1 AND user_id = $2
        RETURNING id, user_id, session_key, channel, chat_id, title,
                  last_inbound_at, cancel_requested, created_at
        "#,
    )
    .bind(session_id)
    .bind(user_id)
    .bind(title)
    .fetch_optional(pool)
    .await
}

pub async fn delete_owned(
    pool: &PgPool,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM sessions WHERE id = $1 AND user_id = $2")
        .bind(session_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn touch_last_inbound(pool: &PgPool, session_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET last_inbound_at = NOW() WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}
