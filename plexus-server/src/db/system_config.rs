use crate::error::ApiError;
use plexus_common::{ErrorCode, LlmApiKey};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeMap;

const MAX_CONCURRENCY_LIMIT: i64 = 1_000_000;

pub const SUPPORTED_CONFIG_KEYS: &[&str] = &[
    "quota_bytes",
    "shared_workspace_quota_bytes",
    "llm_max_context_tokens",
    "llm_compaction_threshold_tokens",
    "llm_max_concurrent_requests",
    "llm_endpoint",
    "llm_api_key",
    "llm_model",
];

pub const LLM_IDENTITY_KEYS: &[&str] = &["llm_endpoint", "llm_api_key", "llm_model"];

pub async fn seed_defaults(_pool: &PgPool) -> Result<(), sqlx::Error> {
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
        if !SUPPORTED_CONFIG_KEYS.contains(&key.as_str()) {
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
        "llm_max_concurrent_requests" => non_negative_i64(key, value)
            .and_then(validate_concurrency_limit)
            .map(|_| ()),
        "llm_endpoint" | "llm_model" => non_empty_string(key, value),
        "llm_api_key" => {
            non_empty_string(key, value)?;
            if value.as_str() == Some(crate::openai::REDACTED_LLM_API_KEY) {
                return Err(ApiError::invalid_args(
                    "llm_api_key cannot be the redaction marker",
                ));
            }
            Ok(())
        }
        _ => Err(ApiError::invalid_args(format!(
            "unsupported config key: {key}"
        ))),
    }
}

pub fn identity_changed(values: &BTreeMap<String, Value>) -> bool {
    LLM_IDENTITY_KEYS
        .iter()
        .any(|key| values.contains_key(*key))
}

pub fn redact_for_response(mut values: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    if values.contains_key("llm_api_key") {
        values.insert(
            "llm_api_key".to_string(),
            json!(crate::openai::REDACTED_LLM_API_KEY),
        );
    }
    values
}

pub fn concurrency_limit(values: &BTreeMap<String, Value>) -> Option<i64> {
    values
        .get("llm_max_concurrent_requests")
        .and_then(Value::as_i64)
}

pub async fn current_concurrency_limit(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let value: Option<Value> = sqlx::query_scalar(
        "SELECT value FROM system_config WHERE key = 'llm_max_concurrent_requests'",
    )
    .fetch_optional(pool)
    .await?;
    Ok(value.and_then(|value| value.as_i64()).unwrap_or(0))
}

pub async fn current_llm_config(pool: &PgPool) -> Result<crate::openai::OpenAiConfig, ApiError> {
    let current = get_all(pool).await.map_err(ApiError::from_sqlx)?;
    merged_llm_config(&current, &BTreeMap::new())
}

pub fn merged_llm_config(
    current: &BTreeMap<String, Value>,
    patch: &BTreeMap<String, Value>,
) -> Result<crate::openai::OpenAiConfig, ApiError> {
    let endpoint = merged_string(current, patch, "llm_endpoint")?;
    let api_key = merged_string(current, patch, "llm_api_key")?;
    let model = merged_string(current, patch, "llm_model")?;

    if api_key == crate::openai::REDACTED_LLM_API_KEY {
        return Err(ApiError::invalid_args(
            "llm_api_key cannot be the redaction marker",
        ));
    }

    let endpoint = endpoint
        .parse::<reqwest::Url>()
        .map_err(|_| ApiError::invalid_args("llm_endpoint must be a valid URL"))?;
    if !matches!(endpoint.scheme(), "http" | "https") || !endpoint.has_host() {
        return Err(ApiError::invalid_args(
            "llm_endpoint must be an http or https URL with a host",
        ));
    }

    Ok(crate::openai::OpenAiConfig {
        endpoint,
        api_key: LlmApiKey::new(api_key),
        model,
    })
}

fn merged_string(
    current: &BTreeMap<String, Value>,
    patch: &BTreeMap<String, Value>,
    key: &str,
) -> Result<String, ApiError> {
    let value = patch.get(key).or_else(|| current.get(key)).ok_or_else(|| {
        ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            ErrorCode::InvalidArgs,
            format!("{key} is required when configuring LLM provider"),
        )
    })?;
    let value = value
        .as_str()
        .ok_or_else(|| ApiError::invalid_args(format!("{key} must be a string")))?;
    if value.trim().is_empty() {
        return Err(ApiError::invalid_args(format!("{key} must not be empty")));
    }
    Ok(value.to_string())
}

fn non_empty_string(key: &str, value: &Value) -> Result<(), ApiError> {
    let value = value
        .as_str()
        .ok_or_else(|| ApiError::invalid_args(format!("{key} must be a string")))?;
    if value.trim().is_empty() {
        return Err(ApiError::invalid_args(format!("{key} must not be empty")));
    }
    Ok(())
}

fn non_negative_i64(key: &str, value: &Value) -> Result<i64, ApiError> {
    let n = value
        .as_i64()
        .ok_or_else(|| ApiError::invalid_args(format!("{key} must be an integer")))?;
    if n < 0 {
        return Err(ApiError::invalid_args(format!(
            "{key} must be zero or positive"
        )));
    }
    Ok(n)
}

fn validate_concurrency_limit(value: i64) -> Result<i64, ApiError> {
    if value > MAX_CONCURRENCY_LIMIT {
        return Err(ApiError::invalid_args(format!(
            "llm_max_concurrent_requests must be at most {MAX_CONCURRENCY_LIMIT}"
        )));
    }
    Ok(value)
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
