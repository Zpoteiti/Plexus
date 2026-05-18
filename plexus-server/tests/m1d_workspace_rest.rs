mod support;

use axum::http::{Method, StatusCode, header};
use serde_json::json;
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

#[tokio::test]
async fn file_routes_require_explicit_server_device() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, _) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/files/notes.txt",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = support::empty_request(
        app.router.clone(),
        Method::GET,
        "/api/workspace/files/notes.txt?plexus_device=devbox",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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
