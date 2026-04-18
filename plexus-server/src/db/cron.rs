use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Value of `cron_jobs.kind` for server-managed jobs (e.g., Plan D's "dream").
/// See C-3, C-4, and C-5 — all three sites compare against this const
/// rather than repeat the "system" literal.
pub const SYSTEM_KIND: &str = "system";

/// Value of `cron_jobs.kind` for normal user-created jobs (the default).
/// Used by Plan D INSERT callers — suppressed until C-5 consumes it.
#[allow(dead_code)]
pub const USER_KIND: &str = "user";

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CronJob {
    pub job_id: String,
    pub user_id: String,
    pub name: String,
    pub enabled: bool,
    pub cron_expr: Option<String>,
    pub every_seconds: Option<i32>,
    pub timezone: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    pub delete_after_run: bool,
    pub deliver: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub run_count: i32,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    /// "user" (default) or "system" — system jobs are managed by the server
    /// and cannot be removed by users.
    pub kind: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_job(
    pool: &PgPool,
    job_id: &str,
    user_id: &str,
    name: &str,
    cron_expr: Option<String>,
    every_seconds: Option<i32>,
    timezone: &str,
    message: &str,
    channel: &str,
    chat_id: &str,
    delete_after_run: bool,
    deliver: bool,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO cron_jobs \
         (job_id, user_id, name, cron_expr, every_seconds, timezone, message, \
          channel, chat_id, delete_after_run, deliver, next_run_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(job_id)
    .bind(user_id)
    .bind(name)
    .bind(cron_expr)
    .bind(every_seconds)
    .bind(timezone)
    .bind(message)
    .bind(channel)
    .bind(chat_id)
    .bind(delete_after_run)
    .bind(deliver)
    .bind(next_run_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_user(pool: &PgPool, user_id: &str) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>(
        "SELECT * FROM cron_jobs WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn find_by_id(pool: &PgPool, job_id: &str) -> Result<Option<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>("SELECT * FROM cron_jobs WHERE job_id = $1")
        .bind(job_id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_job(
    pool: &PgPool,
    job_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM cron_jobs WHERE job_id = $1 AND user_id = $2",
    )
    .bind(job_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Atomically claim all due jobs: sets next_run_at = NULL and claimed_at = NOW().
/// Returns the claimed jobs. Any job returned here is exclusively owned by this
/// server instance until reschedule_job / unclaim_job clears claimed_at.
/// Safe to call from multiple server nodes simultaneously — each UPDATE is atomic.
pub async fn claim_due_jobs(pool: &PgPool) -> Result<Vec<CronJob>, sqlx::Error> {
    sqlx::query_as::<_, CronJob>(
        "UPDATE cron_jobs \
         SET claimed_at = NOW(), next_run_at = NULL \
         WHERE enabled = true \
           AND next_run_at IS NOT NULL \
           AND next_run_at <= NOW() \
         RETURNING *",
    )
    .fetch_all(pool)
    .await
}

/// Called by the agent loop after successfully completing a cron event turn.
/// Sets the next run time and clears claimed_at so the poller can pick it up again.
pub async fn reschedule_job(
    pool: &PgPool,
    job_id: &str,
    next_run_at: Option<DateTime<Utc>>,
    success: bool,
) -> Result<(), sqlx::Error> {
    let status = if success { "ok" } else { "error" };
    sqlx::query(
        "UPDATE cron_jobs \
         SET last_run_at = NOW(), \
             run_count = run_count + 1, \
             next_run_at = $1, \
             claimed_at = NULL, \
             last_status = $2 \
         WHERE job_id = $3",
    )
    .bind(next_run_at)
    .bind(status)
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Called when the bus fails to dispatch a claimed job.
/// Resets the job to retry in 1 minute instead of waiting for stuck recovery.
pub async fn unclaim_job(pool: &PgPool, job_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cron_jobs \
         SET claimed_at = NULL, \
             next_run_at = NOW() + INTERVAL '1 minute' \
         WHERE job_id = $1",
    )
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Recovery sweep: any job that has been claimed for > 30 minutes without
/// rescheduling is assumed to be from a crashed server. Reset it to run soon.
/// Returns the number of jobs recovered.
pub async fn recover_stuck_jobs(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE cron_jobs \
         SET claimed_at = NULL, \
             next_run_at = NOW() + INTERVAL '1 minute', \
             last_status = 'recovered' \
         WHERE next_run_at IS NULL \
           AND claimed_at IS NOT NULL \
           AND claimed_at < NOW() - INTERVAL '30 minutes' \
           AND enabled = true",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Disable a one-shot job after execution (at-mode, not delete_after_run).
pub async fn disable_job(pool: &PgPool, job_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cron_jobs \
         SET enabled = false, next_run_at = NULL, claimed_at = NULL \
         WHERE job_id = $1",
    )
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_job_struct_compiles_with_new_fields() {
        let j = CronJob {
            job_id: "test".into(),
            user_id: "u1".into(),
            name: "my job".into(),
            enabled: true,
            cron_expr: None,
            every_seconds: Some(60),
            timezone: "UTC".into(),
            message: "hello".into(),
            channel: "gateway".into(),
            chat_id: "chat1".into(),
            delete_after_run: false,
            deliver: true,
            next_run_at: None,
            last_run_at: None,
            run_count: 0,
            created_at: chrono::Utc::now(),
            claimed_at: None,
            last_status: None,
            kind: "user".into(),
        };
        assert!(j.claimed_at.is_none());
        assert!(j.last_status.is_none());
    }
}
