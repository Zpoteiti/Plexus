//! Cron job CRUD API endpoints.

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, patch};
use axum::{Json, Router};
use plexus_common::errors::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

async fn list_jobs(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let jobs = crate::db::cron::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(serde_json::json!({ "cron_jobs": jobs })))
}

#[derive(Deserialize)]
struct CreateJobRequest {
    name: Option<String>,
    message: String,
    cron_expr: Option<String>,
    every_seconds: Option<i32>,
    at: Option<String>,
    channel: Option<String>,
    chat_id: Option<String>,
    timezone: Option<String>,
    delete_after_run: Option<bool>,
    deliver: Option<bool>,
}

async fn create_job(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateJobRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    let c = claims(&headers, &state)?;

    let name = req
        .name
        .unwrap_or_else(|| req.message.chars().take(30).collect());
    let channel = req
        .channel
        .unwrap_or_else(|| plexus_common::consts::CHANNEL_GATEWAY.into());
    let chat_id = req.chat_id.unwrap_or_default();
    let timezone = req.timezone.unwrap_or_else(|| "UTC".into());
    let delete_after_run = req.delete_after_run.unwrap_or(req.at.is_some());
    let deliver = req.deliver.unwrap_or(true);

    // Validate timezone
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            format!("Invalid timezone: {timezone}"),
        ));
    }

    // Validate scheduling mode
    let modes = [
        req.cron_expr.is_some(),
        req.every_seconds.is_some(),
        req.at.is_some(),
    ];
    if modes.iter().filter(|&&b| b).count() != 1 {
        return Err(ApiError::new(
            ErrorCode::ValidationFailed,
            "Exactly one of cron_expr, every_seconds, or at must be specified",
        ));
    }

    let job_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let now = chrono::Utc::now();

    // Compute next_run_at
    let next_run_at = if let Some(ref expr) = req.cron_expr {
        Some(
            crate::server_tools::cron_tool::compute_next_cron_pub(expr, &timezone)
                .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, e))?,
        )
    } else if let Some(secs) = req.every_seconds {
        Some(now + chrono::Duration::seconds(secs as i64))
    } else if let Some(ref at) = req.at {
        Some(
            crate::server_tools::cron_tool::parse_at_datetime(at, &timezone)
                .map_err(|e| ApiError::new(ErrorCode::ValidationFailed, e))?,
        )
    } else {
        None
    };

    crate::db::cron::create_job(
        &state.db,
        &job_id,
        &c.sub,
        &name,
        req.cron_expr,
        req.every_seconds,
        &timezone,
        &req.message,
        &channel,
        &chat_id,
        delete_after_run,
        deliver,
        next_run_at,
    )
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "job_id": job_id })),
    ))
}

#[derive(Deserialize)]
struct UpdateJobRequest {
    enabled: Option<bool>,
    message: Option<String>,
    name: Option<String>,
}

async fn update_job(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    Json(req): Json<UpdateJobRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    // Simple update — just enabled/message/name fields
    // Build dynamic UPDATE query
    let mut sets = Vec::new();
    let mut bind_idx = 1;

    if req.enabled.is_some() {
        sets.push(format!("enabled = ${bind_idx}"));
        bind_idx += 1;
    }
    if req.message.is_some() {
        sets.push(format!("message = ${bind_idx}"));
        bind_idx += 1;
    }
    if req.name.is_some() {
        sets.push(format!("name = ${bind_idx}"));
        bind_idx += 1;
    }

    if sets.is_empty() {
        return Ok(Json(serde_json::json!({ "message": "Nothing to update" })));
    }

    let sql = format!(
        "UPDATE cron_jobs SET {} WHERE job_id = ${bind_idx} AND user_id = ${}",
        sets.join(", "),
        bind_idx + 1
    );

    let mut query = sqlx::query(&sql);
    if let Some(enabled) = req.enabled {
        query = query.bind(enabled);
    }
    if let Some(ref message) = req.message {
        query = query.bind(message);
    }
    if let Some(ref name) = req.name {
        query = query.bind(name);
    }
    query = query.bind(&job_id).bind(&c.sub);

    query
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;

    Ok(Json(serde_json::json!({ "message": "Cron job updated" })))
}

async fn delete_job(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    let c = claims(&headers, &state)?;

    // Load job first — needed for ownership + kind checks before any DELETE.
    let job = crate::db::cron::find_by_id(&state.db, &job_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("DB: {e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Cron job not found"))?;

    // Ownership check: return NotFound (not Forbidden) to avoid an enumeration oracle.
    if job.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::NotFound, "Cron job not found"));
    }

    // Kind guard: system jobs are server-managed and cannot be removed via the API.
    if job.kind == crate::db::cron::SYSTEM_KIND {
        return Err(ApiError::new(
            ErrorCode::Forbidden,
            "Cannot remove system cron jobs (these are managed by the server).",
        ));
    }

    crate::db::cron::delete_job(&state.db, &job_id, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("Delete: {e}")))?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn cron_api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/cron-jobs", get(list_jobs).post(create_job))
        .route(
            "/api/cron-jobs/{job_id}",
            patch(update_job).delete(delete_job),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires DATABASE_URL. Verifies that the HTTP DELETE handler refuses system-kind jobs.
    /// Run with: cargo test --package plexus-server cron_api -- --ignored
    #[tokio::test]
    #[ignore]
    async fn test_delete_cron_refuses_system_job() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set to run this test");
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("failed to connect to test DB");

        let user_id = format!("delc4-sys-{}", uuid::Uuid::new_v4());
        let user_email = format!("{user_id}@test.local");
        sqlx::query(
            "INSERT INTO users (user_id, username, email, password_hash, is_admin) \
             VALUES ($1, $1, $2, '', false) ON CONFLICT DO NOTHING",
        )
        .bind(&user_id)
        .bind(&user_email)
        .execute(&pool)
        .await
        .expect("insert test user");

        let job_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        sqlx::query(
            "INSERT INTO cron_jobs \
             (job_id, user_id, name, timezone, message, channel, chat_id, \
              delete_after_run, deliver, kind) \
             VALUES ($1, $2, 'dream', 'UTC', '', 'gateway', '-', false, false, 'system')",
        )
        .bind(&job_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("insert system cron job");

        let tmp = tempfile::TempDir::new().unwrap();
        let state = crate::state::AppState::test_with_pool(pool.clone(), tmp.path());

        // Build a fake JWT for the test user
        let secret = &state.config.jwt_secret;
        let claims_obj = crate::auth::Claims {
            sub: user_id.clone(),
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            is_admin: false,
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims_obj,
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("encode JWT");

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );

        let result = delete_job(
            headers,
            axum::extract::State(state.clone()),
            axum::extract::Path(job_id.clone()),
        )
        .await;

        assert!(result.is_err(), "expected Err for system job delete");
        let err = result.unwrap_err();
        assert_eq!(
            err.code, "FORBIDDEN",
            "expected FORBIDDEN code, got: {}",
            err.code
        );
        assert!(
            err.message.contains("system"),
            "expected 'system' in message, got: {}",
            err.message
        );

        // Row must still exist.
        let still = crate::db::cron::find_by_id(&pool, &job_id).await.unwrap();
        assert!(still.is_some(), "system job must not have been deleted");

        // Cleanup
        sqlx::query("DELETE FROM cron_jobs WHERE job_id = $1")
            .bind(&job_id)
            .execute(&pool)
            .await
            .ok();
        sqlx::query("DELETE FROM users WHERE user_id = $1")
            .bind(&user_id)
            .execute(&pool)
            .await
            .ok();
    }

    /// Requires DATABASE_URL. Verifies that user-kind jobs CAN be deleted via the HTTP endpoint.
    /// Run with: cargo test --package plexus-server cron_api -- --ignored
    #[tokio::test]
    #[ignore]
    async fn test_delete_cron_allows_user_job() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set to run this test");
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("failed to connect to test DB");

        let user_id = format!("delc4-usr-{}", uuid::Uuid::new_v4());
        let user_email = format!("{user_id}@test.local");
        sqlx::query(
            "INSERT INTO users (user_id, username, email, password_hash, is_admin) \
             VALUES ($1, $1, $2, '', false) ON CONFLICT DO NOTHING",
        )
        .bind(&user_id)
        .bind(&user_email)
        .execute(&pool)
        .await
        .expect("insert test user");

        let job_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        sqlx::query(
            "INSERT INTO cron_jobs \
             (job_id, user_id, name, timezone, message, channel, chat_id, \
              delete_after_run, deliver, kind) \
             VALUES ($1, $2, 'my-job', 'UTC', 'hello', 'gateway', '-', false, false, 'user')",
        )
        .bind(&job_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("insert user cron job");

        let tmp = tempfile::TempDir::new().unwrap();
        let state = crate::state::AppState::test_with_pool(pool.clone(), tmp.path());

        let secret = &state.config.jwt_secret;
        let claims_obj = crate::auth::Claims {
            sub: user_id.clone(),
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            is_admin: false,
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims_obj,
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("encode JWT");

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );

        let result = delete_job(
            headers,
            axum::extract::State(state.clone()),
            axum::extract::Path(job_id.clone()),
        )
        .await;

        assert!(
            result.is_ok(),
            "expected Ok for user job delete, got: {result:?}"
        );
        assert_eq!(result.unwrap(), axum::http::StatusCode::NO_CONTENT,);

        // Row must be gone.
        let gone = crate::db::cron::find_by_id(&pool, &job_id).await.unwrap();
        assert!(gone.is_none(), "user job should have been deleted");

        // Cleanup (user only; job is gone)
        sqlx::query("DELETE FROM users WHERE user_id = $1")
            .bind(&user_id)
            .execute(&pool)
            .await
            .ok();
    }
}
