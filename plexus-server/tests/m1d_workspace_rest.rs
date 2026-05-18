mod support;

use axum::http::{Method, StatusCode, header};
use plexus_server::routes::workspace::WORKSPACE_REST_UPLOAD_MEMORY_LIMIT_BYTES;
use serde_json::{Value, json};
use support::TestApp;

async fn set_quota(app: &TestApp, quota: i64) {
    sqlx::query(
        "INSERT INTO system_config (key, value, updated_at)
         VALUES ('quota_bytes', $1, NOW())
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(json!(quota))
    .execute(&app.pool)
    .await
    .unwrap();
}

fn error_body(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

#[tokio::test]
async fn file_routes_require_explicit_server_device() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, body) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/files/notes.txt",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_body(&body)["code"], "invalid_args");

    let (status, body) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/files/notes.txt?plexus_device=devbox",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_body(&body)["code"], "invalid_args");
}

#[tokio::test]
async fn put_get_delete_file_round_trip() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;
    let path = "/api/workspace/files/.attachments/uploads/abc/cat.txt?plexus_device=server";

    let (status, _, _) = support::bytes_request(
        app.router.clone(),
        Method::PUT,
        path,
        b"cat".to_vec(),
        "application/octet-stream",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, headers, body) = support::bytes_request(
        app.router.clone(),
        Method::GET,
        path,
        Vec::new(),
        "application/octet-stream",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/octet-stream"
    );
    assert_eq!(body, b"cat");

    let (status, _) =
        support::empty_request(app.router.clone(), Method::DELETE, path, Some(&jwt)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn edit_list_glob_grep_and_folder_delete_work() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let (status, _, _) = support::bytes_request(
        app.router.clone(),
        Method::PUT,
        "/api/workspace/files/docs/a.txt?plexus_device=server",
        b"hello world".to_vec(),
        "application/octet-stream",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/workspace/files/docs/a.txt?plexus_device=server",
        json!({
            "old_text": "world",
            "new_text": "plexus",
            "replace_all": false,
        }),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["replacements"], 1);

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/list/docs?plexus_device=server",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_array().unwrap().iter().any(|entry| {
        entry["name"] == "a.txt"
            && entry["path"] == "docs/a.txt"
            && entry["kind"] == "file"
            && entry["size"] == 12
    }));

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/glob?plexus_device=server&pattern=docs/*.txt",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!(["docs/a.txt"]));

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/grep?plexus_device=server&pattern=plexus&path=docs",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|line| line == "docs/a.txt:1:hello plexus")
    );

    let (status, _) = support::empty_request(
        app.router.clone(),
        Method::DELETE,
        "/api/workspace/folders/docs?plexus_device=server",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn glob_route_requires_explicit_server_device() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, body) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/glob?pattern=docs/*.txt",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_body(&body)["code"], "invalid_args");
}

#[tokio::test]
async fn quota_route_reports_server_workspace_usage() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/quota",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["quota_bytes"], 10_000);
    assert_eq!(body["locked"], false);
}

#[tokio::test]
async fn missing_file_returns_not_found_code_without_server_path() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, body) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/files/notes.txt?plexus_device=server",
        Some(&jwt),
    )
    .await;
    let body = error_body(&body);
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
    assert!(
        !body["message"]
            .as_str()
            .unwrap()
            .contains(app.db_name.as_str())
    );
    assert!(!body["message"].as_str().unwrap().contains("/"));
}

#[tokio::test]
async fn path_traversal_returns_forbidden_code_without_server_path() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, body) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/files/%2E%2E/secret.txt?plexus_device=server",
        Some(&jwt),
    )
    .await;
    let body = error_body(&body);
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "path_outside_workspace");
    assert!(!body["message"].as_str().unwrap().contains("/"));
}

#[tokio::test]
async fn quota_route_returns_quota_not_configured_code() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, body) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/quota",
        Some(&jwt),
    )
    .await;
    let body = error_body(&body);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "quota_not_configured");
}

#[tokio::test]
async fn oversized_upload_returns_upload_too_large_code() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let (status, _, body) = support::bytes_request(
        app.router.clone(),
        Method::PUT,
        "/api/workspace/files/large.bin?plexus_device=server",
        vec![b'x'; 8_001],
        "application/octet-stream",
        Some(&jwt),
    )
    .await;
    let body = error_body(&body);
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "upload_too_large");
}

#[tokio::test]
async fn locked_workspace_put_returns_soft_locked_before_upload_size_check() {
    let app = TestApp::spawn().await;
    let (jwt, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;
    let workspace = support::workspace_path(&app.workspace_root, user_id);
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::write(workspace.join("existing.bin"), vec![b'x'; 10_001])
        .await
        .unwrap();

    let (status, _, body) = support::bytes_request(
        app.router.clone(),
        Method::PUT,
        "/api/workspace/files/large.bin?plexus_device=server",
        vec![b'x'; 8_001],
        "application/octet-stream",
        Some(&jwt),
    )
    .await;
    let body = error_body(&body);
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "soft_locked");
}

#[tokio::test]
async fn upload_above_axum_default_body_limit_can_succeed() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 4_000_000).await;

    let (status, _, _) = support::bytes_request(
        app.router.clone(),
        Method::PUT,
        "/api/workspace/files/big.bin?plexus_device=server",
        vec![b'x'; 2_200_000],
        "application/octet-stream",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn upload_above_rest_memory_limit_returns_upload_too_large_code() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 100_000_000).await;

    let (status, _, body) = support::bytes_request(
        app.router.clone(),
        Method::PUT,
        "/api/workspace/files/too-large-for-rest.bin?plexus_device=server",
        vec![b'x'; WORKSPACE_REST_UPLOAD_MEMORY_LIMIT_BYTES as usize + 1],
        "application/octet-stream",
        Some(&jwt),
    )
    .await;
    let body = error_body(&body);
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "upload_too_large");
}
