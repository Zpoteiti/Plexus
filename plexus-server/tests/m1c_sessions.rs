mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::{Value, json};
use support::{TestApp, json_request_with_headers as json_request};
use tower::ServiceExt;
use uuid::Uuid;

async fn register(app: axum::Router, email: &str) -> String {
    let (status, _, body) = json_request(
        app,
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
    body["jwt"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn create_web_session_defaults_title_and_sets_web_key() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "alice@example.com").await;

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();
    Uuid::parse_str(id).unwrap();
    assert_eq!(body["title"], "New chat");
    assert_eq!(body["channel"], "web");
    assert_eq!(body["chat_id"], id);
    assert_eq!(body["session_key"], format!("web:{id}"));
}

#[tokio::test]
async fn create_list_read_rename_and_delete_session() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "alice@example.com").await;

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({"title": "  Journey to Japan  "}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["title"], "Journey to Japan");
    let id = body["id"].as_str().unwrap();

    let (status, _, list) = json_request(
        app.router.clone(),
        Method::GET,
        "/api/sessions",
        Value::Null,
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::GET,
        &format!("/api/sessions/{id}"),
        Value::Null,
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], id);

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        &format!("/api/sessions/{id}"),
        json!({"title": "Japan itinerary"}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "Japan itinerary");

    let (status, _, _) = json_request(
        app.router.clone(),
        Method::PATCH,
        &format!("/api/sessions/{id}"),
        json!({"title": "   "}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/sessions/{id}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let (status, _, _) = json_request(
        app.router.clone(),
        Method::GET,
        &format!("/api/sessions/{id}"),
        Value::Null,
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_ownership_returns_404_for_other_users() {
    let app = TestApp::spawn().await;
    let alice = register(app.router.clone(), "alice@example.com").await;
    let bob = register(app.router.clone(), "bob@example.com").await;

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(&alice),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();

    let (status, _, _) = json_request(
        app.router.clone(),
        Method::GET,
        &format!("/api/sessions/{id}"),
        Value::Null,
        Some(&bob),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_message_accepts_unspecified_reasoning_effort() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "alice@example.com").await;
    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{id}/messages"),
        json!({"content": [{"type": "text", "text": "hello"}], "attachments": []}),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body["message_id"].as_str().is_some());
}

#[tokio::test]
async fn post_message_accepts_null_reasoning_effort() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "alice-null@example.com").await;
    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{id}/messages"),
        json!({
            "content": [{"type": "text", "text": "hello"}],
            "attachments": [],
            "reasoning_effort": null
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body["message_id"].as_str().is_some());
}

#[tokio::test]
async fn post_message_rejects_invalid_reasoning_effort() {
    let app = TestApp::spawn().await;
    let token = register(app.router.clone(), "alice2@example.com").await;
    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{id}/messages"),
        json!({
            "content": [{"type": "text", "text": "hello"}],
            "attachments": [],
            "reasoning_effort": "off"
        }),
        Some(&token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("reasoning_effort must be one of")
    );
}
