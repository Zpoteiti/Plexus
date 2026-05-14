mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use plexus_common::ErrorCode;
use plexus_server::{
    chat::prompt,
    db::{sessions, system_config, users},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use support::{TestApp, fake_openai::FakeOpenAi};
use tower::ServiceExt;
use uuid::Uuid;

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
        "name": "Alice"
    });
    if admin {
        body["admin_token"] = json!("test-admin-token");
    }
    let (status, body) = json_request(app, Method::POST, "/api/auth/register", body, None).await;
    assert_eq!(status, StatusCode::CREATED);
    body["jwt"].as_str().unwrap().to_string()
}

async fn configure_llm(app: &TestApp, token: &str, fake: &FakeOpenAi) {
    let (status, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/admin/config",
        json!({
            "llm_endpoint": fake.base_url,
            "llm_api_key": fake.api_key(),
            "llm_model": fake.model(),
            "llm_max_concurrent_requests": 0
        }),
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn create_session(app: &TestApp, token: &str) -> Uuid {
    let (status, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
}

async fn post_text(app: &TestApp, token: &str, session_id: Uuid, text: &str) {
    let (status, _) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({
            "content": [{"type": "text", "text": text}],
            "reasoning_effort": "medium"
        }),
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

async fn wait_for_assistant(app: &TestApp, session_id: Uuid) -> Value {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some((content,)) = sqlx::query_as::<_, (Value,)>(
            "SELECT content FROM messages
             WHERE session_id = $1 AND role = 'assistant'
             ORDER BY created_at DESC
             LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&app.pool)
        .await
        .unwrap()
        {
            return content;
        }
        assert!(Instant::now() < deadline, "assistant message timed out");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_assistant_with_reasoning(
    app: &TestApp,
    session_id: Uuid,
) -> (Value, Option<String>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some((content, reasoning_content)) = sqlx::query_as::<_, (Value, Option<String>)>(
            "SELECT content, reasoning_content FROM messages
             WHERE session_id = $1 AND role = 'assistant'
             ORDER BY created_at DESC
             LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&app.pool)
        .await
        .unwrap()
        {
            return (content, reasoning_content);
        }
        assert!(Instant::now() < deadline, "assistant message timed out");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_chat_calls(fake: &FakeOpenAi, min_calls: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while fake.chat_call_count() < min_calls {
        assert!(Instant::now() < deadline, "fake provider call timed out");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn first_text(content: &Value) -> &str {
    content[0]["text"].as_str().unwrap()
}

#[tokio::test]
async fn prompt_reads_optional_soul_and_memory() {
    let app = TestApp::spawn().await;
    let user = users::create_user(&app.pool, "alice@example.com", "hash", "Alice", false)
        .await
        .unwrap();
    let user_dir = app.workspace_root.path().join(user.id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::write(user_dir.join("SOUL.md"), "Be concise.")
        .await
        .unwrap();
    tokio::fs::write(user_dir.join("MEMORY.md"), "Alice likes trains.")
        .await
        .unwrap();
    let session = sessions::create_web_session(&app.pool, user.id, "New chat")
        .await
        .unwrap();

    let text = prompt::build_system_prompt(app.workspace_root.path(), &user, &session)
        .await
        .unwrap();
    assert!(text.contains("## SOUL"));
    assert!(text.contains("Be concise."));
    assert!(text.contains("## MEMORY"));
    assert!(text.contains("Alice likes trains."));
    assert!(text.contains("M1c has no tools available"));
}

#[tokio::test]
async fn stored_llm_config_requires_identity_values() {
    let app = TestApp::spawn().await;
    let err = system_config::current_llm_config(&app.pool)
        .await
        .expect_err("missing config should reject");
    assert_eq!(err.code, ErrorCode::InvalidArgs);

    let mut values = std::collections::BTreeMap::new();
    values.insert(
        "llm_endpoint".to_string(),
        json!("http://127.0.0.1:1234/v1"),
    );
    values.insert("llm_api_key".to_string(), json!("test-key"));
    values.insert("llm_model".to_string(), json!("test-model"));
    let mut tx = app.pool.begin().await.unwrap();
    system_config::set_many(&mut tx, &values).await.unwrap();
    tx.commit().await.unwrap();

    let cfg = system_config::current_llm_config(&app.pool)
        .await
        .expect("stored config");
    assert_eq!(cfg.model, "test-model");
}

#[tokio::test]
async fn post_message_calls_fake_provider_and_persists_assistant() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "admin@example.com", true).await;
    let fake = FakeOpenAi::valid().await;
    configure_llm(&app, &token, &fake).await;
    let session_id = create_session(&app, &token).await;

    post_text(&app, &token, session_id, "hello").await;

    let content = wait_for_assistant(&app, session_id).await;
    assert_eq!(first_text(&content), "hi");
}

#[tokio::test]
async fn native_reasoning_content_is_persisted_and_returned_in_history() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "admin-reasoning@example.com", true).await;
    let fake = FakeOpenAi::reasoning_content().await;
    configure_llm(&app, &token, &fake).await;
    let session_id = create_session(&app, &token).await;

    post_text(&app, &token, session_id, "hello").await;

    let (content, reasoning_content) = wait_for_assistant_with_reasoning(&app, session_id).await;
    assert_eq!(first_text(&content), "visible answer");
    assert_eq!(reasoning_content.as_deref(), Some("native reasoning"));

    let (status, body) = json_request(
        app.router.clone(),
        Method::GET,
        &format!("/api/sessions/{session_id}/messages"),
        Value::Null,
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let assistant = body
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["reasoning_content"], "native reasoning");
}

#[tokio::test]
async fn leading_think_block_is_extracted_before_persistence() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "admin-think-tag@example.com", true).await;
    let fake = FakeOpenAi::think_tagged_content().await;
    configure_llm(&app, &token, &fake).await;
    let session_id = create_session(&app, &token).await;

    post_text(&app, &token, session_id, "hello").await;

    let (content, reasoning_content) = wait_for_assistant_with_reasoning(&app, session_id).await;
    assert_eq!(first_text(&content), "visible answer");
    assert_eq!(reasoning_content.as_deref(), Some("tag reasoning"));
}

#[tokio::test]
async fn missing_llm_config_persists_synthetic_assistant_message() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "alice@example.com", false).await;
    let session_id = create_session(&app, &token).await;

    post_text(&app, &token, session_id, "hello").await;

    let content = wait_for_assistant(&app, session_id).await;
    assert!(first_text(&content).contains("Plexus could not complete the LLM request"));
    assert!(first_text(&content).contains("llm_endpoint is required"));
}

#[tokio::test]
async fn provider_failure_message_is_secret_free() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "admin@example.com", true).await;
    let fake = FakeOpenAi::always_unavailable().await;
    configure_llm(&app, &token, &fake).await;
    let session_id = create_session(&app, &token).await;

    post_text(&app, &token, session_id, "hello").await;

    let content = wait_for_assistant(&app, session_id).await;
    let text = first_text(&content);
    assert!(text.contains("HTTP 529"));
    assert!(!text.contains(fake.api_key()));
    assert!(!text.contains("Bearer"));
}

#[tokio::test]
async fn concurrent_posts_to_one_session_do_not_create_parallel_fake_provider_calls() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "admin@example.com", true).await;
    let fake = FakeOpenAi::delayed(Duration::from_millis(150)).await;
    configure_llm(&app, &token, &fake).await;
    let session_id = create_session(&app, &token).await;

    let (left, right) = tokio::join!(
        post_text(&app, &token, session_id, "hello"),
        post_text(&app, &token, session_id, "ping"),
    );
    let _ = (left, right);

    let _content = wait_for_assistant(&app, session_id).await;
    wait_for_chat_calls(&fake, 1).await;
    assert_eq!(fake.max_in_flight(), 1);
}

#[tokio::test]
async fn image_compatibility_failure_retries_stripped_and_persists_assistant() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "admin@example.com", true).await;
    let fake = FakeOpenAi::image_unsupported_then_valid().await;
    configure_llm(&app, &token, &fake).await;
    let session_id = create_session(&app, &token).await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({
            "reasoning_effort": "medium",
            "content": [
                {"type": "text", "text": "what is this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8="}}
            ]
        }),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let content = wait_for_assistant(&app, session_id).await;
    assert_eq!(first_text(&content), "image stripped fallback");
    assert_eq!(fake.chat_call_count(), 2);
}
