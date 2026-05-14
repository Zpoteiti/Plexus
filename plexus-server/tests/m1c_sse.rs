mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::Response,
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
        json!({"content": [{"type": "text", "text": text}]}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    (token, session_id)
}

async fn read_sse_until(
    response: Response,
    expected: &str,
    timeout: std::time::Duration,
) -> String {
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
        json!({"content": [{"type": "text", "text": "live hello"}]}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let text = reader.await.unwrap();
    let history_end = text.find("event: history_end").unwrap();
    let live_message = text.find("live hello").unwrap();
    assert!(history_end < live_message);
}
