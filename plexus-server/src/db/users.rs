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

pub async fn get_last_heartbeat_at(
    pool: &PgPool,
    user_id: &str,
) -> sqlx::Result<Option<DateTime<Utc>>> {
    let row: Option<(Option<DateTime<Utc>>,)> =
        sqlx::query_as("SELECT last_heartbeat_at FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(v,)| v))
}

pub async fn update_last_heartbeat_at(
    pool: &PgPool,
    user_id: &str,
    at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET last_heartbeat_at = $1 WHERE user_id = $2")
        .bind(at)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Return user_ids whose last_heartbeat_at is NULL or older than
/// `threshold_seconds` in the past. Ordered oldest-first (NULL counts as
/// epoch, so never-fired users come before stale-fired users). Bounded by
/// `limit` so a single tick can't OOM the server on an interval shrink.
pub async fn list_users_due_for_heartbeat(
    pool: &PgPool,
    threshold_seconds: i64,
    limit: i64,
) -> sqlx::Result<Vec<String>> {
    // INTERVAL '1 second' * $1::bigint is the portable form for "N seconds
    // where N is an i64" — make_interval would need an ::integer cast.
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT user_id FROM users \
         WHERE last_heartbeat_at IS NULL \
            OR last_heartbeat_at < NOW() - (INTERVAL '1 second' * $1::bigint) \
         ORDER BY COALESCE(last_heartbeat_at, 'epoch'::timestamptz) ASC \
         LIMIT $2",
    )
    .bind(threshold_seconds)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
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

    #[tokio::test]
    #[ignore] // needs DATABASE_URL
    async fn test_last_heartbeat_at_roundtrip() {
        let url = std::env::var("DATABASE_URL")
            .expect("set DATABASE_URL to run this test");
        let pool = crate::db::init_db(&url).await;

        let user_id = format!("e1-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let user_email = format!("{user_id}@test.local");
        crate::db::users::create_user(&pool, &user_id, &user_email, "", false)
            .await
            .unwrap();

        let initial = get_last_heartbeat_at(&pool, &user_id).await.unwrap();
        assert!(initial.is_none(), "fresh user should have NULL last_heartbeat_at");

        let now = chrono::Utc::now();
        update_last_heartbeat_at(&pool, &user_id, now).await.unwrap();

        let after = get_last_heartbeat_at(&pool, &user_id).await.unwrap()
            .expect("timestamp should be present after update");
        // Postgres TIMESTAMPTZ stores microsecond precision; allow a 5ms tolerance.
        let delta = (after - now).num_milliseconds().abs();
        assert!(delta < 5, "expected roundtrip within 5ms; got {delta}ms delta");

        sqlx::query("DELETE FROM users WHERE user_id = $1")
            .bind(&user_id)
            .execute(&pool)
            .await
            .ok();
    }

    #[tokio::test]
    #[ignore] // needs DATABASE_URL
    async fn test_list_users_due_for_heartbeat_selects_null_and_stale() {
        let url = std::env::var("DATABASE_URL")
            .expect("set DATABASE_URL to run this test");
        let pool = crate::db::init_db(&url).await;

        // Three users: one NULL, one stale (1h ago), one fresh (10s ago).
        let ids: Vec<String> = (0..3)
            .map(|i| format!("e1d-{}-{}", i, &uuid::Uuid::new_v4().to_string()[..8]))
            .collect();
        for id in &ids {
            crate::db::users::create_user(
                &pool,
                id,
                &format!("{id}@test.local"),
                "",
                false,
            )
            .await
            .unwrap();
        }

        // ids[0] stays NULL. ids[1] is stale. ids[2] is fresh.
        update_last_heartbeat_at(
            &pool,
            &ids[1],
            chrono::Utc::now() - chrono::Duration::hours(1),
        )
        .await
        .unwrap();
        update_last_heartbeat_at(&pool, &ids[2], chrono::Utc::now())
            .await
            .unwrap();

        // Threshold 1800s = 30 minutes. NULL + 1h-ago are due; 10s-ago is not.
        let due = list_users_due_for_heartbeat(&pool, 1800, 10).await.unwrap();
        assert!(due.contains(&ids[0]), "NULL user should be due");
        assert!(due.contains(&ids[1]), "stale user should be due");
        assert!(!due.contains(&ids[2]), "fresh user should NOT be due");

        // Oldest-first ordering: NULL (epoch) < 1h-ago.
        let pos_null = due.iter().position(|x| x == &ids[0]).unwrap();
        let pos_stale = due.iter().position(|x| x == &ids[1]).unwrap();
        assert!(pos_null < pos_stale, "NULL user should sort before stale user");

        for id in &ids {
            sqlx::query("DELETE FROM users WHERE user_id = $1")
                .bind(id)
                .execute(&pool)
                .await
                .ok();
        }
    }
}
