mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use http_body_util::BodyExt;
use plexus_common::ContentBlock;
use plexus_server::db::messages;
use serde_json::{Value, json};
use std::time::Duration;
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

async fn register_create_and_post(app: &TestApp, text: &str) -> (String, String) {
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
    let session_id = body["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({
            "content": [{"type": "text", "text": text}],
            "reasoning_effort": "medium"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    (token, session_id)
}

async fn register(app: &TestApp, email: &str, admin: bool) -> String {
    let mut body = json!({
        "email": email,
        "password": "correct horse battery staple",
        "name": "Alice"
    });
    if admin {
        body["admin_token"] = json!("test-admin-token");
    }
    let (status, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/register",
        body,
        None,
    )
    .await;
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

async fn post_text(app: &TestApp, token: &str, session_id: Uuid, text: &str) -> String {
    let (status, body) = json_request(
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
    body["message_id"].as_str().unwrap().to_string()
}

async fn read_sse_until(response: Response, expected: &str, timeout: Duration) -> String {
    let mut body = response.into_body();
    let deadline = std::time::Instant::now() + timeout;
    let mut out = String::new();
    while std::time::Instant::now() < deadline {
        let Some(frame) = tokio::time::timeout(std::time::Duration::from_millis(100), body.frame())
            .await
            .ok()
            .flatten()
        else {
            continue;
        };
        let frame = frame.unwrap();
        if let Some(bytes) = frame.data_ref() {
            out.push_str(std::str::from_utf8(bytes).unwrap());
            if out.contains(expected) {
                return out;
            }
        }
    }
    panic!("timed out waiting for SSE text {expected:?}; got {out}");
}

async fn next_sse_frame_text(body: &mut Body, timeout: Duration) -> Option<String> {
    let frame = tokio::time::timeout(timeout, body.frame())
        .await
        .ok()??
        .unwrap();
    frame
        .data_ref()
        .map(|bytes| std::str::from_utf8(bytes).unwrap().to_string())
}

#[tokio::test]
async fn sse_replays_messages_then_history_end() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_create_and_post(&app, "hello").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/stream?replay_limit=50"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = read_sse_until(
        response,
        "event: history_end",
        std::time::Duration::from_secs(1),
    )
    .await;
    assert!(text.contains("event: message"));
    assert!(text.contains("event: history_end"));
    assert!(text.contains("hello"));
}

#[tokio::test]
async fn sse_emits_live_message_after_history_end() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_create_and_post(&app, "old").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/stream?replay_limit=0"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let reader = tokio::spawn(read_sse_until(
        response,
        "live hello",
        std::time::Duration::from_secs(1),
    ));

    let (status, _) = json_request(
        app.router.clone(),
        Method::POST,
        &format!("/api/sessions/{session_id}/messages"),
        json!({
            "content": [{"type": "text", "text": "live hello"}],
            "reasoning_effort": "medium"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let text = reader.await.unwrap();
    let history_end = text.find("event: history_end").unwrap();
    let live_message = text.find("live hello").unwrap();
    assert!(history_end < live_message);
}

#[tokio::test]
async fn sse_live_stream_receives_user_and_assistant_messages() {
    let app = TestApp::spawn().await;
    let token = register(&app, "admin@example.com", true).await;
    let fake = FakeOpenAi::valid().await;
    configure_llm(&app, &token, &fake).await;
    let session_id = create_session(&app, &token).await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/stream?replay_limit=0"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let reader = tokio::spawn(read_sse_until(
        response,
        "\"text\":\"hi\"",
        std::time::Duration::from_secs(2),
    ));
    post_text(&app, &token, session_id, "hello").await;

    let text = reader.await.unwrap();
    assert!(text.contains("event: history_end"));
    assert!(text.contains("hello"));
    assert!(text.contains("\"text\":\"hi\""));
}

#[tokio::test]
async fn last_event_id_replays_only_newer_messages() {
    let app = TestApp::spawn().await;
    let token = register(&app, "alice@example.com", false).await;
    let session_id = create_session(&app, &token).await;
    let first_id = post_text(&app, &token, session_id, "first marker").await;
    post_text(&app, &token, session_id, "second marker").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/stream?replay_limit=50"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("last-event-id", first_id)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let text = read_sse_until(
        response,
        "event: history_end",
        std::time::Duration::from_secs(1),
    )
    .await;
    assert!(text.contains("second marker"));
    assert!(!text.contains("first marker"));
}

#[tokio::test]
async fn sse_skips_live_duplicate_for_message_already_replayed() {
    let app = TestApp::spawn().await;
    let token = register(&app, "alice@example.com", false).await;
    let session_id = create_session(&app, &token).await;
    let message = messages::insert_message(
        &app.pool,
        session_id,
        "user",
        vec![ContentBlock::text("duplicate marker")],
    )
    .await
    .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/stream?replay_limit=50"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    app.state.chat().broker().broadcast(message).await;
    let mut body = response.into_body();

    let replay = next_sse_frame_text(&mut body, Duration::from_secs(1))
        .await
        .expect("replayed message frame");
    assert!(replay.contains("duplicate marker"));
    let history_end = next_sse_frame_text(&mut body, Duration::from_secs(1))
        .await
        .expect("history_end frame");
    assert!(history_end.contains("event: history_end"));

    let duplicate = next_sse_frame_text(&mut body, Duration::from_millis(100)).await;
    assert!(
        duplicate.is_none(),
        "message already sent in replay must not be emitted again as live SSE"
    );
}

#[tokio::test]
async fn sse_stream_closes_after_receiver_lag() {
    let app = TestApp::spawn().await;
    let token = register(&app, "alice@example.com", false).await;
    let session_id = create_session(&app, &token).await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/stream?replay_limit=0"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();

    let history_end = next_sse_frame_text(&mut body, Duration::from_secs(1))
        .await
        .expect("history_end frame");
    assert!(history_end.contains("event: history_end"));

    for index in 0..260 {
        let message = messages::insert_message(
            &app.pool,
            session_id,
            "user",
            vec![ContentBlock::text(format!("lag marker {index}"))],
        )
        .await
        .unwrap();
        app.state.chat().broker().broadcast(message).await;
    }

    let next = tokio::time::timeout(Duration::from_secs(1), body.frame())
        .await
        .expect("lagged stream should close promptly");
    assert!(next.is_none(), "receiver lag must close the SSE stream");
}
