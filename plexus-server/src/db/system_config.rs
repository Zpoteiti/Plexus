use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeMap;

pub const SUPPORTED_M1A_KEYS: &[&str] = &[
    "quota_bytes",
    "shared_workspace_quota_bytes",
    "llm_max_context_tokens",
    "llm_compaction_threshold_tokens",
    "llm_max_concurrent_requests",
];

pub const DEFERRED_LLM_IDENTITY_KEYS: &[&str] = &["llm_endpoint", "llm_api_key", "llm_model"];

pub async fn seed_defaults(pool: &PgPool) -> Result<(), sqlx::Error> {
    let defaults = [
        ("quota_bytes", json!(5_i64 * 1024 * 1024 * 1024)),
        ("shared_workspace_quota_bytes", json!(25_i64 * 1024 * 1024 * 1024)),
        ("llm_max_context_tokens", json!(128000)),
        ("llm_compaction_threshold_tokens", json!(16000)),
        ("llm_max_concurrent_requests", Value::Null),
    ];

    for (key, value) in defaults {
        sqlx::query(
            "INSERT INTO system_config (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn get_all(pool: &PgPool) -> Result<BTreeMap<String, Value>, sqlx::Error> {
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT key, value FROM system_config ORDER BY key")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

pub async fn set_many(
    tx: &mut Transaction<'_, Postgres>,
    values: &BTreeMap<String, Value>,
) -> Result<(), sqlx::Error> {
    for (key, value) in values {
        sqlx::query(
            "INSERT INTO system_config (key, value, updated_at)
             VALUES ($1, $2, NOW())
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
        )
        .bind(key)
        .bind(value)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
