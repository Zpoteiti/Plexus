mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use support::TestApp;
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

async fn register_and_create_session(app: &TestApp) -> (String, String) {
    let (status, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/register",
        json!({
            "email": "alice@example.com",
            "password": "correct horse battery staple",
            "name": "Alice"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = body["jwt"].as_str().unwrap().to_string();

    let (status, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (token, body["id"].as_str().unwrap().to_string())
}

async fn register_user(app: &TestApp, email: &str) -> (String, Uuid) {
    let (status, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/register",
        json!({
            "email": email,
            "password": "correct horse battery staple",
            "name": "Alice"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["jwt"].as_str().unwrap().to_string(),
        Uuid::parse_str(body["user"]["id"].as_str().unwrap()).unwrap(),
    )
}

#[tokio::test]
async fn post_text_message_persists_runtime_and_content_blocks() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let (status, body) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({"content": [{"type": "text", "text": "hello"}]}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let message_id = body["message_id"].as_str().unwrap();

    let stored: (serde_json::Value,) = sqlx::query_as("SELECT content FROM messages WHERE id = $1")
        .bind(uuid::Uuid::parse_str(message_id).unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let blocks = stored.0.as_array().unwrap();
    assert!(blocks[0]["text"].as_str().unwrap().contains("<runtime>"));
    assert_eq!(blocks[1], json!({"type": "text", "text": "hello"}));
}

#[tokio::test]
async fn post_empty_forms_are_accepted() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;
    for body in [json!({}), json!({"content": ""}), json!({"content": []})] {
        let (status, _) = json_request(
            app.router.clone(),
            Method::POST,
            &format!("/api/sessions/{session_id}/messages"),
            body,
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }
}

#[tokio::test]
async fn inline_base64_image_is_accepted_but_external_url_is_rejected() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let (status, _) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({"content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8="}}]}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({"content": [{"type": "image_url", "image_url": {"url": "https://example.com/cat.png"}}]}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn browser_post_to_non_web_owned_session_is_bad_request() {
    let app = TestApp::spawn().await;
    let (token, user_id) = register_user(&app, "alice@example.com").await;
    let session_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO sessions (id, user_id, session_key, channel, chat_id, title)
        VALUES ($1, $2, $3, 'discord', $4, 'Discord')
        "#,
    )
    .bind(session_id)
    .bind(user_id)
    .bind("discord:dm:12345")
    .bind("12345")
    .execute(&app.pool)
    .await
    .unwrap();

    let (status, _) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({"content": [{"type": "text", "text": "hello"}]}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
