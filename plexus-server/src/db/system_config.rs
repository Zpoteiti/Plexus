use crate::error::ApiError;
use axum::http::StatusCode;
use plexus_common::ErrorCode;
use serde_json::{Value, json};
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
        (
            "shared_workspace_quota_bytes",
            json!(25_i64 * 1024 * 1024 * 1024),
        ),
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

pub fn validate_patch(input: BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, ApiError> {
    let mut out = BTreeMap::new();
    for (key, value) in input {
        if DEFERRED_LLM_IDENTITY_KEYS.contains(&key.as_str()) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidArgs,
                format!("{key} requires M1b provider validation before DB write"),
            ));
        }
        if !SUPPORTED_M1A_KEYS.contains(&key.as_str()) {
            return Err(ApiError::invalid_args(format!(
                "unsupported config key: {key}"
            )));
        }
        validate_value(&key, &value)?;
        out.insert(key, value);
    }
    Ok(out)
}

fn validate_value(key: &str, value: &Value) -> Result<(), ApiError> {
    match key {
        "quota_bytes" | "shared_workspace_quota_bytes" | "llm_max_context_tokens" => {
            positive_i64(key, value).map(|_| ())
        }
        "llm_compaction_threshold_tokens" => {
            let value = positive_i64(key, value)?;
            if value <= 4000 {
                return Err(ApiError::invalid_args(
                    "llm_compaction_threshold_tokens must be greater than 4000",
                ));
            }
            Ok(())
        }
        "llm_max_concurrent_requests" => {
            if value.is_null() {
                return Ok(());
            }
            positive_i64(key, value).map(|_| ())
        }
        _ => Err(ApiError::invalid_args(format!(
            "unsupported config key: {key}"
        ))),
    }
}

fn positive_i64(key: &str, value: &Value) -> Result<i64, ApiError> {
    let n = value
        .as_i64()
        .ok_or_else(|| ApiError::invalid_args(format!("{key} must be an integer")))?;
    if n <= 0 {
        return Err(ApiError::invalid_args(format!("{key} must be positive")));
    }
    Ok(n)
}
