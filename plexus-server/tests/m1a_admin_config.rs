mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use support::TestApp;
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

async fn register(app: axum::Router, email: &str, admin: bool) -> String {
    let mut body = json!({
        "email": email,
        "password": "correct horse battery staple",
        "name": email
    });
    if admin {
        body["admin_token"] = json!("test-admin-token");
    }

    let (status, body) = json_request(app, Method::POST, "/api/auth/register", body, None).await;
    assert_eq!(status, StatusCode::CREATED);
    body["jwt"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn admin_config_requires_admin() {
    let app = TestApp::spawn().await;
    let user_token = register(app.router.clone(), "user@example.com", false).await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::GET,
        "/api/admin/config",
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = json_request(
        app.router.clone(),
        Method::GET,
        "/api/admin/config",
        Value::Null,
        Some(&user_token),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_patch_supported_config_keys() {
    let app = TestApp::spawn().await;
    let admin_token = register(app.router.clone(), "admin@example.com", true).await;

    let (status, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "quota_bytes": 12345,
            "shared_workspace_quota_bytes": 67890,
            "llm_max_context_tokens": 128000,
            "llm_compaction_threshold_tokens": 16000,
            "llm_max_concurrent_requests": 32
        }),
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["quota_bytes"], 12345);
    assert_eq!(body["llm_max_concurrent_requests"], 32);

    let stored: (serde_json::Value,) =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'quota_bytes'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(stored.0, json!(12345));

    let (status, body) = json_request(
        app.router.clone(),
        Method::GET,
        "/api/admin/config",
        Value::Null,
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["quota_bytes"], 12345);
}

#[tokio::test]
async fn unsupported_or_deferred_keys_reject_atomically() {
    let app = TestApp::spawn().await;
    let admin_token = register(app.router.clone(), "admin@example.com", true).await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({ "quota_bytes": 999, "llm_model": "gpt-test" }),
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let stored: (serde_json::Value,) =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'quota_bytes'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_ne!(stored.0, json!(999));

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({ "unknown_key": 1 }),
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
