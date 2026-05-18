mod support;

use axum::http::{Method, StatusCode};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use support::{TestApp, json_request, register_user, workspace_path};
use uuid::Uuid;

async fn register_and_create_session(app: &TestApp) -> (String, String) {
    let (token, _) = register_user(app, "alice@example.com").await;
    let session_id = create_session(app, &token).await;
    (token, session_id)
}

async fn create_session(app: &TestApp, token: &str) -> String {
    let (status, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/sessions",
        json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["id"].as_str().unwrap().to_string()
}

async fn set_workspace_quota(app: &TestApp) {
    sqlx::query(
        r#"
        INSERT INTO system_config (key, value, updated_at)
        VALUES ('quota_bytes', $1, NOW())
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()
        "#,
    )
    .bind(json!(1_000_000))
    .execute(&app.pool)
    .await
    .unwrap();
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

async fn stored_message_content(app: &TestApp, message_id: &str) -> Value {
    let stored: (Value,) = sqlx::query_as("SELECT content FROM messages WHERE id = $1")
        .bind(Uuid::parse_str(message_id).unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    stored.0
}

async fn user_message_count(app: &TestApp, session_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = $1 AND role = 'user'")
        .bind(Uuid::parse_str(session_id).unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

fn png_bytes() -> Vec<u8> {
    vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, b'p', b'l', b'x',
    ]
}

fn jpeg_bytes() -> Vec<u8> {
    vec![0xff, 0xd8, 0xff, b'p', b'l', b'x']
}

fn gif_bytes() -> Vec<u8> {
    b"GIF89aplx".to_vec()
}

fn webp_bytes() -> Vec<u8> {
    b"RIFF0000WEBPplx".to_vec()
}

fn data_image_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
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

#[tokio::test]
async fn missing_attachment_device_or_file_rejects_whole_message() {
    let app = TestApp::spawn().await;
    let (token, session_id) = register_and_create_session(&app).await;

    let cases = [
        (
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"path": ".attachments/uploads/a/cat.png"}]
            }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"plexus_device": "server", "path": ".attachments/uploads/a/cat.png", "extra": true}]
            }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"plexus_device": 123, "path": ".attachments/uploads/a/cat.png"}]
            }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"plexus_device": "server", "path": 123}]
            }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"plexus_device": "server", "path": "/tmp/cat.png"}]
            }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"plexus_device": "devbox", "path": ".attachments/uploads/a/cat.png"}]
            }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"plexus_device": "server", "path": ".attachments/uploads/a/missing.png"}]
            }),
            StatusCode::NOT_FOUND,
        ),
    ];

    for (body, expected_status) in cases {
        let (status, _) = post_message(&app, &token, &session_id, body).await;
        assert_eq!(status, expected_status);
        assert_eq!(user_message_count(&app, &session_id).await, 0);
    }
}

#[tokio::test]
async fn directory_attachment_ref_is_forbidden_and_rejects_whole_message() {
    let app = TestApp::spawn().await;
    let (token, user_id) = register_user(&app, "alice@example.com").await;
    let session_id = create_session(&app, &token).await;
    tokio::fs::create_dir_all(
        workspace_path(&app.workspace_root, user_id).join(".attachments/uploads/a/dir"),
    )
    .await
    .unwrap();

    let (status, body) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [{"type": "text", "text": "hello"}],
            "attachments": [{"plexus_device": "server", "path": ".attachments/uploads/a/dir"}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "path_outside_workspace");
    assert_eq!(user_message_count(&app, &session_id).await, 0);
}

#[tokio::test]
async fn non_image_attachment_adds_marker_only_before_user_content() {
    let app = TestApp::spawn().await;
    let (token, user_id) = register_user(&app, "alice@example.com").await;
    let session_id = create_session(&app, &token).await;
    set_workspace_quota(&app).await;
    app.state
        .workspace_fs()
        .write_file(
            user_id,
            ".attachments/uploads/a/readme.txt",
            b"not an image".to_vec(),
        )
        .await
        .unwrap();

    let (status, body) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [{"type": "text", "text": "hello"}],
            "attachments": [{"plexus_device": "server", "path": ".attachments/uploads/a/readme.txt"}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let stored = stored_message_content(&app, body["message_id"].as_str().unwrap()).await;
    let blocks = stored.as_array().unwrap();
    assert!(blocks[0]["text"].as_str().unwrap().contains("<runtime>"));
    assert_eq!(
        blocks[1],
        json!({"type": "text", "text": "User uploaded file to device='server', path=\".attachments/uploads/a/readme.txt\""})
    );
    assert_eq!(blocks[2], json!({"type": "text", "text": "hello"}));
    assert!(blocks.iter().all(|block| block["type"] != "image_url"));
}

#[tokio::test]
async fn image_attachment_adds_marker_then_generated_image_before_user_content() {
    let app = TestApp::spawn().await;
    let (token, user_id) = register_user(&app, "alice@example.com").await;
    let session_id = create_session(&app, &token).await;
    set_workspace_quota(&app).await;
    let bytes = png_bytes();
    app.state
        .workspace_fs()
        .write_file(user_id, ".attachments/uploads/a/cat.png", bytes.clone())
        .await
        .unwrap();

    let (status, body) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [{"type": "text", "text": "hello"}],
            "attachments": [{"plexus_device": "server", "path": ".attachments/uploads/a/cat.png"}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let stored = stored_message_content(&app, body["message_id"].as_str().unwrap()).await;
    let blocks = stored.as_array().unwrap();
    assert!(blocks[0]["text"].as_str().unwrap().contains("<runtime>"));
    assert_eq!(
        blocks[1],
        json!({"type": "text", "text": "User uploaded file to device='server', path=\".attachments/uploads/a/cat.png\""})
    );
    assert_eq!(
        blocks[2],
        json!({"type": "image_url", "image_url": {"url": data_image_url("image/png", &bytes)}})
    );
    assert_eq!(blocks[3], json!({"type": "text", "text": "hello"}));
}

#[tokio::test]
async fn duplicate_direct_image_gets_marker_inserted_before_existing_image() {
    let app = TestApp::spawn().await;
    let (token, user_id) = register_user(&app, "alice@example.com").await;
    let session_id = create_session(&app, &token).await;
    set_workspace_quota(&app).await;
    let bytes = png_bytes();
    let direct_url = data_image_url("image/png", &bytes);
    app.state
        .workspace_fs()
        .write_file(user_id, ".attachments/uploads/a/cat.png", bytes)
        .await
        .unwrap();

    let (status, body) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [
                {"type": "text", "text": "描述这张图"},
                {"type": "image_url", "image_url": {"url": direct_url}}
            ],
            "attachments": [{"plexus_device": "server", "path": ".attachments/uploads/a/cat.png"}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let stored = stored_message_content(&app, body["message_id"].as_str().unwrap()).await;
    let blocks = stored.as_array().unwrap();
    assert_eq!(blocks[1], json!({"type": "text", "text": "描述这张图"}));
    assert_eq!(
        blocks[2],
        json!({"type": "text", "text": "User uploaded file to device='server', path=\".attachments/uploads/a/cat.png\""})
    );
    assert_eq!(
        blocks[3],
        json!({"type": "image_url", "image_url": {"url": direct_url}})
    );
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block["type"] == "image_url")
            .count(),
        1
    );
}

#[tokio::test]
async fn non_duplicate_direct_image_keeps_attachment_image_and_user_image() {
    let app = TestApp::spawn().await;
    let (token, user_id) = register_user(&app, "alice@example.com").await;
    let session_id = create_session(&app, &token).await;
    set_workspace_quota(&app).await;
    let attachment_bytes = png_bytes();
    let direct_bytes = b"different inline bytes";
    app.state
        .workspace_fs()
        .write_file(
            user_id,
            ".attachments/uploads/a/cat.png",
            attachment_bytes.clone(),
        )
        .await
        .unwrap();

    let (status, body) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": data_image_url("image/png", direct_bytes)}}
            ],
            "attachments": [{"plexus_device": "server", "path": ".attachments/uploads/a/cat.png"}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let stored = stored_message_content(&app, body["message_id"].as_str().unwrap()).await;
    let blocks = stored.as_array().unwrap();
    assert_eq!(
        blocks[1],
        json!({"type": "text", "text": "User uploaded file to device='server', path=\".attachments/uploads/a/cat.png\""})
    );
    assert_eq!(
        blocks[2],
        json!({"type": "image_url", "image_url": {"url": data_image_url("image/png", &attachment_bytes)}})
    );
    assert_eq!(blocks[3], json!({"type": "text", "text": "hello"}));
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block["type"] == "image_url")
            .count(),
        2
    );
}

#[tokio::test]
async fn attachment_marker_escapes_path_text() {
    let app = TestApp::spawn().await;
    let (token, user_id) = register_user(&app, "alice@example.com").await;
    let session_id = create_session(&app, &token).await;
    set_workspace_quota(&app).await;
    let path = ".attachments/uploads/a/quote'\"\n.txt";
    app.state
        .workspace_fs()
        .write_file(user_id, path, b"not an image".to_vec())
        .await
        .unwrap();

    let (status, body) = post_message(
        &app,
        &token,
        &session_id,
        json!({
            "reasoning_effort": null,
            "content": [{"type": "text", "text": "hello"}],
            "attachments": [{"plexus_device": "server", "path": path}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let stored = stored_message_content(&app, body["message_id"].as_str().unwrap()).await;
    let blocks = stored.as_array().unwrap();
    assert_eq!(
        blocks[1],
        json!({"type": "text", "text": "User uploaded file to device='server', path=\".attachments/uploads/a/quote'\\\"\\n.txt\""})
    );
}

#[tokio::test]
async fn non_png_image_attachments_use_sniffed_mime_type() {
    let cases = [
        (".attachments/uploads/a/cat.jpg", "image/jpeg", jpeg_bytes()),
        (".attachments/uploads/a/cat.gif", "image/gif", gif_bytes()),
        (
            ".attachments/uploads/a/cat.webp",
            "image/webp",
            webp_bytes(),
        ),
    ];

    for (path, mime, bytes) in cases {
        let app = TestApp::spawn().await;
        let (token, user_id) = register_user(&app, "alice@example.com").await;
        let session_id = create_session(&app, &token).await;
        set_workspace_quota(&app).await;
        app.state
            .workspace_fs()
            .write_file(user_id, path, bytes.clone())
            .await
            .unwrap();

        let (status, body) = post_message(
            &app,
            &token,
            &session_id,
            json!({
                "reasoning_effort": null,
                "content": [{"type": "text", "text": "hello"}],
                "attachments": [{"plexus_device": "server", "path": path}]
            }),
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        let stored = stored_message_content(&app, body["message_id"].as_str().unwrap()).await;
        let blocks = stored.as_array().unwrap();
        assert_eq!(
            blocks[2],
            json!({"type": "image_url", "image_url": {"url": data_image_url(mime, &bytes)}})
        );
    }
}
