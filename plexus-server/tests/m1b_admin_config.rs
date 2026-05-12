mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use support::{TestApp, fake_openai::FakeOpenAi};
use tower::ServiceExt;

async fn json_request(
    app: axum::Router,
    method: Method,
    path: &str,
    body: Value,
    auth: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = auth {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }

    let response = app
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

async fn register_admin(app: axum::Router) -> String {
    let (status, body) = json_request(
        app,
        Method::POST,
        "/api/auth/register",
        json!({
            "email": "admin@example.com",
            "password": "correct horse battery staple",
            "name": "Admin",
            "admin_token": "test-admin-token"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["jwt"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn admin_can_set_valid_llm_identity_after_models_validation() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;
    let fake = FakeOpenAi::valid().await;

    let (status, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "llm_endpoint": fake.base_url,
            "llm_api_key": fake.api_key(),
            "llm_model": fake.model(),
            "llm_max_concurrent_requests": 0
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["llm_endpoint"], fake.base_url);
    assert_eq!(body["llm_api_key"], "<redacted>");
    assert_eq!(body["llm_model"], fake.model());

    let stored: (serde_json::Value,) =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'llm_api_key'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(stored.0, json!(fake.api_key()));
}

#[tokio::test]
async fn invalid_llm_identity_patch_is_atomic() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;
    let fake = FakeOpenAi::missing_model().await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "quota_bytes": 999,
            "llm_endpoint": fake.base_url,
            "llm_api_key": fake.api_key(),
            "llm_model": fake.model()
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let quota: (serde_json::Value,) =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'quota_bytes'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_ne!(quota.0, json!(999));
}

#[tokio::test]
async fn first_llm_identity_setup_requires_all_three_keys() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;
    let fake = FakeOpenAi::valid().await;

    let (status, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "llm_endpoint": fake.base_url,
            "llm_model": fake.model()
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_args");
}

#[tokio::test]
async fn llm_identity_patch_can_reuse_stored_key() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;
    let fake = FakeOpenAi::valid().await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "llm_endpoint": fake.base_url,
            "llm_api_key": fake.api_key(),
            "llm_model": fake.model()
        }),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({"llm_model": fake.model()}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["llm_api_key"], "<redacted>");
}

#[tokio::test]
async fn redaction_marker_is_rejected_as_new_api_key() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;
    let fake = FakeOpenAi::valid().await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "llm_endpoint": fake.base_url,
            "llm_api_key": "<redacted>",
            "llm_model": fake.model()
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invalid_concurrency_limit_is_rejected_before_persistence() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({"llm_max_concurrent_requests": 1_000_001}),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let stored: (serde_json::Value,) =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'llm_max_concurrent_requests'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(stored.0, json!(0));
}
