use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub user_id: String,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn create_user(
    pool: &PgPool,
    user_id: &str,
    email: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (user_id, email, password_hash, is_admin) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(email)
    .bind(password_hash)
    .bind(is_admin)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(
        "SELECT user_id, email, password_hash, is_admin, display_name, created_at FROM users WHERE email = $1"
    )
    .bind(email)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_id(pool: &PgPool, user_id: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(
        "SELECT user_id, email, password_hash, is_admin, display_name, created_at FROM users WHERE user_id = $1"
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn update_display_name(
    pool: &PgPool,
    user_id: &str,
    display_name: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET display_name = $1 WHERE user_id = $2")
        .bind(display_name)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_timezone(pool: &PgPool, user_id: &str, tz: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET timezone = $1 WHERE user_id = $2")
        .bind(tz)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn get_timezone(pool: &PgPool, user_id: &str) -> sqlx::Result<String> {
    sqlx::query_scalar::<_, String>("SELECT timezone FROM users WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

pub async fn get_last_dream_at(
    pool: &PgPool,
    user_id: &str,
) -> sqlx::Result<Option<DateTime<Utc>>> {
    let row: Option<(Option<DateTime<Utc>>,)> =
        sqlx::query_as("SELECT last_dream_at FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(v,)| v))
}

pub async fn update_last_dream_at(
    pool: &PgPool,
    user_id: &str,
    at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET last_dream_at = $1 WHERE user_id = $2")
        .bind(at)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // needs DATABASE_URL
    async fn test_last_dream_at_roundtrip() {
        let url = std::env::var("DATABASE_URL")
            .expect("set DATABASE_URL to run this test");
        let pool = crate::db::init_db(&url).await;

        let user_id = format!("d3-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let user_email = format!("{user_id}@test.local");
        crate::db::users::create_user(
            &pool, &user_id, &user_email, "", false,
        ).await.unwrap();

        // Initial state: None (column default NULL).
        let initial = get_last_dream_at(&pool, &user_id).await.unwrap();
        assert!(initial.is_none(), "freshly-created user should have last_dream_at=NULL");

        // Write a timestamp.
        let now = chrono::Utc::now();
        update_last_dream_at(&pool, &user_id, now).await.unwrap();

        // Read it back.
        let after = get_last_dream_at(&pool, &user_id).await.unwrap()
            .expect("timestamp should be present after update");

        // Postgres TIMESTAMPTZ stores microsecond precision; allow a 5ms tolerance.
        let delta = (after - now).num_milliseconds().abs();
        assert!(delta < 5, "expected roundtrip within 5ms; got {delta}ms delta");

        // Cleanup.
        sqlx::query("DELETE FROM users WHERE user_id = $1")
            .bind(&user_id)
            .execute(&pool)
            .await
            .ok();
    }
}
