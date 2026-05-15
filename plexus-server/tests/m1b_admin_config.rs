mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use plexus_common::{AdminToken, ChatRole, ContentBlock, JwtSecret, LlmApiKey, ReasoningEffort};
use plexus_server::{
    app::{self as server_app, AppState},
    config::ServerConfig,
    openai::{ChatCompletionRequest, ChatMessage, OpenAiConfig, OpenAiRuntime},
};
use serde_json::{Value, json};
use std::time::Duration;
use support::{TestApp, fake_openai::FakeOpenAi};
use tower::ServiceExt;
use url::Url;

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

fn llm_config(fake: &FakeOpenAi) -> OpenAiConfig {
    OpenAiConfig {
        endpoint: fake.base_url.parse().unwrap(),
        api_key: LlmApiKey::new(fake.api_key().to_string()),
        model: fake.model().to_string(),
    }
}

fn chat_request() -> ChatCompletionRequest {
    ChatCompletionRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ContentBlock::text("ping")],
            reasoning_content: None,
        }],
        max_tokens: None,
        temperature: None,
        reasoning_effort: Some(ReasoningEffort::Medium),
    }
}

fn router_with_runtime(app: &TestApp, runtime: OpenAiRuntime) -> axum::Router {
    let mut database_url = Url::parse(&app.admin_url).expect("valid admin database URL");
    database_url.set_path(&format!("/{}", app.db_name));

    let cfg = ServerConfig {
        database_url: database_url.to_string(),
        workspace_root: app.workspace_root.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        jwt_secret: JwtSecret::new("test-jwt-secret-with-enough-entropy".to_string()),
        admin_token: Some(AdminToken::new("test-admin-token".to_string())),
        cookie_secure: false,
    };

    server_app::router(AppState::new_with_openai_runtime(
        app.pool.clone(),
        cfg,
        runtime,
    ))
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
async fn get_config_redacts_configured_llm_api_key() {
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
        Method::GET,
        "/api/admin/config",
        Value::Null,
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["llm_api_key"], "<redacted>");
    assert!(!body.to_string().contains(fake.api_key()));
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
    let quota: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'quota_bytes'")
            .fetch_optional(&app.pool)
            .await
            .unwrap();
    assert!(quota.is_none());
}

#[tokio::test]
async fn invalid_llm_endpoint_is_rejected_before_persistence() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;
    let fake = FakeOpenAi::valid().await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "quota_bytes": 999,
            "llm_endpoint": "ftp://example.com/v1",
            "llm_api_key": fake.api_key(),
            "llm_model": fake.model()
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let quota: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'quota_bytes'")
            .fetch_optional(&app.pool)
            .await
            .unwrap();
    assert!(quota.is_none());
}

#[tokio::test]
async fn llm_endpoint_with_allowed_scheme_but_missing_host_is_rejected_before_persistence() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;
    let fake = FakeOpenAi::valid().await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "quota_bytes": 999,
            "llm_endpoint": "https:///v1",
            "llm_api_key": fake.api_key(),
            "llm_model": fake.model()
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let quota: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'quota_bytes'")
            .fetch_optional(&app.pool)
            .await
            .unwrap();
    assert!(quota.is_none());
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
async fn failed_concurrency_persistence_does_not_mutate_runtime_limit() {
    let app = TestApp::spawn().await;
    let runtime = OpenAiRuntime::default();
    let router = router_with_runtime(&app, runtime.clone());
    let token = register_admin(router.clone()).await;

    sqlx::query(
        "CREATE FUNCTION fail_system_config_write() RETURNS trigger
         LANGUAGE plpgsql AS $$
         BEGIN
             RAISE EXCEPTION 'forced system_config write failure';
         END;
         $$",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_system_config_write
         BEFORE INSERT OR UPDATE ON system_config
         FOR EACH ROW EXECUTE FUNCTION fail_system_config_write()",
    )
    .execute(&app.pool)
    .await
    .unwrap();

    let (status, _) = json_request(
        router,
        Method::PATCH,
        "/api/admin/config",
        json!({"llm_max_concurrent_requests": 1}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    let fake = FakeOpenAi::delayed(Duration::from_millis(150)).await;
    let cfg = llm_config(&fake);
    let request = chat_request();
    let (left, right) = tokio::join!(
        runtime.chat_completion(&cfg, request.clone()),
        runtime.chat_completion(&cfg, request),
    );

    left.expect("left response");
    right.expect("right response");
    assert_eq!(fake.max_in_flight(), 2);
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
async fn negative_concurrency_limit_is_rejected_before_persistence() {
    let app = TestApp::spawn().await;
    let token = register_admin(app.router.clone()).await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({"llm_max_concurrent_requests": -1}),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let stored: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'llm_max_concurrent_requests'")
            .fetch_optional(&app.pool)
            .await
            .unwrap();
    assert!(stored.is_none());
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
    let stored: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'llm_max_concurrent_requests'")
            .fetch_optional(&app.pool)
            .await
            .unwrap();
    assert!(stored.is_none());
}

#[tokio::test]
async fn missing_concurrency_limit_is_runtime_unlimited() {
    let app = TestApp::spawn().await;
    let stored: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT value FROM system_config WHERE key = 'llm_max_concurrent_requests'")
            .fetch_optional(&app.pool)
            .await
            .unwrap();
    assert!(stored.is_none());

    let limit = plexus_server::db::system_config::current_concurrency_limit(&app.pool)
        .await
        .unwrap();
    assert_eq!(limit, 0);
}
