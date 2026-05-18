mod support;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use support::{TestApp, json_request, register_user};

async fn register_and_create_session(app: &TestApp) -> (String, String) {
    let (token, _) = register_user(app, "alice@example.com").await;

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

async fn post_message(
    app: &TestApp,
    token: &str,
    session_id: &str,
    body: Value,
) -> (StatusCode, Value) {
    json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        body,
        Some(token),
    )
    .await
}

#[tokio::test]
async fn message_requires_content_and_attachments_arrays() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    for body in [
        json!({"content": []}),
        json!({"attachments": []}),
        json!({"content": "hello", "attachments": []}),
        json!({"content": [], "attachments": [], "extra": true}),
    ] {
        let (status, _) = post_message(&app, &token, &session_id, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn rejects_message_when_both_arrays_are_empty() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let (status, _) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [],
            "attachments": []
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rejects_non_array_attachments() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let (status, _) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [{"type": "text", "text": "hello"}],
            "attachments": "nope"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn accepts_text_and_direct_inline_image_with_empty_attachments() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let (status, body) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8="}}
            ],
            "attachments": []
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body["message_id"].as_str().is_some());
}

#[tokio::test]
async fn rejects_external_direct_image_url() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let (status, _) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [
                {"type": "image_url", "image_url": {"url": "https://example.com/cat.png"}}
            ],
            "attachments": []
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rejects_malformed_inline_image_base64() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let (status, _) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,not valid"}}
            ],
            "attachments": []
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}
