use sqlx::PgPool;

pub async fn get(pool: &PgPool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM system_config WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0))
}

pub async fn set(pool: &PgPool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = NOW()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// Seed default values for workspace-related keys if they don't exist yet.
/// Idempotent — existing admin-edited values are left alone.
pub async fn seed_defaults_if_missing(pool: &PgPool) -> Result<(), sqlx::Error> {
    // Text templates (seeded from shipped workspace templates via include_str!).
    for (key, default) in [
        (
            "default_soul",
            include_str!("../../templates/workspace/SOUL.md"),
        ),
        (
            "default_memory",
            include_str!("../../templates/workspace/MEMORY.md"),
        ),
        (
            "default_heartbeat",
            include_str!("../../templates/workspace/HEARTBEAT.md"),
        ),
    ] {
        if get(pool, key).await?.is_none() {
            set(pool, key, default).await?;
        }
    }

    // Integer / boolean config keys consumed by later plans (D, E).
    for (key, default) in [
        ("workspace_quota_bytes", "5368709120"), // 5 GiB
        ("heartbeat_interval_seconds", "1800"),  // 30 min (Plan E)
        ("dream_enabled", "true"),               // Plan D global kill switch
    ] {
        if get(pool, key).await?.is_none() {
            set(pool, key, default).await?;
        }
    }

    Ok(())
}
