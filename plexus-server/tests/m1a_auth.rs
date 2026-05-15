mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use plexus_server::db::users;
use serde_json::{Value, json};
use support::{TestApp, json_request_with_headers as json_request};
use tower::ServiceExt;

#[tokio::test]
async fn create_user_persists_without_returning_password_hash() {
    let app = TestApp::spawn().await;
    let user = users::create_user(&app.pool, "alice@example.com", "hash-value", "Alice", false)
        .await
        .unwrap();

    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.name, "Alice");
    assert!(!user.is_admin);

    let stored_hash: (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stored_hash.0, "hash-value");
}

#[tokio::test]
async fn register_login_me_patch_and_logout_work_with_real_auth() {
    let app = TestApp::spawn().await;

    let (status, headers, body) = json_request(
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
    let set_cookie = headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(set_cookie.contains("HttpOnly"));
    let jwt = body["jwt"].as_str().unwrap().to_string();
    let user_id = uuid::Uuid::parse_str(body["user"]["id"].as_str().unwrap()).unwrap();
    assert_eq!(body["user"]["email"], "alice@example.com");
    assert!(support::workspace_path(&app.workspace_root, user_id).exists());

    let stored_hash: (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_ne!(stored_hash.0, "correct horse battery staple");

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/login",
        json!({
            "email": "alice@example.com",
            "password": "correct horse battery staple"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["jwt"].as_str().unwrap().len() > 20);

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::GET,
        "/api/me",
        Value::Null,
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "alice@example.com");

    let cookie_response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/me")
                .header(header::COOKIE, set_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cookie_response.status(), StatusCode::OK);

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/me",
        json!({
            "name": "Alice Updated",
            "email": "alice-updated@example.com",
            "password": "new correct horse battery staple"
        }),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Alice Updated");
    assert_eq!(body["email"], "alice-updated@example.com");

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/login",
        json!({
            "email": "alice-updated@example.com",
            "password": "new correct horse battery staple"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["jwt"].as_str().unwrap().len() > 20);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
}

#[tokio::test]
async fn admin_token_creates_admin_user() {
    let app = TestApp::spawn().await;
    let (status, _, body) = json_request(
        app.router.clone(),
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
    assert_eq!(body["user"]["is_admin"], true);
}

#[tokio::test]
async fn duplicate_email_returns_conflict() {
    let app = TestApp::spawn().await;
    for expected in [StatusCode::CREATED, StatusCode::CONFLICT] {
        let (status, _, _) = json_request(
            app.router.clone(),
            Method::POST,
            "/api/auth/register",
            json!({
                "email": "dupe@example.com",
                "password": "correct horse battery staple",
                "name": "Dupe"
            }),
            None,
        )
        .await;
        assert_eq!(status, expected);
    }
}

#[tokio::test]
async fn wrong_password_returns_unauthorized() {
    let app = TestApp::spawn().await;
    let (status, _, _) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/register",
        json!({
            "email": "login@example.com",
            "password": "correct horse battery staple",
            "name": "Login"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _, _) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/login",
        json!({
            "email": "login@example.com",
            "password": "wrong password"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn patch_me_rejects_invalid_profile_fields() {
    let app = TestApp::spawn().await;
    let (status, _, body) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/auth/register",
        json!({
            "email": "profile@example.com",
            "password": "correct horse battery staple",
            "name": "Profile"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let jwt = body["jwt"].as_str().unwrap().to_string();

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/me",
        json!({ "email": "not-an-email" }),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_args");

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/me",
        json!({ "name": "   " }),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_args");

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::GET,
        "/api/me",
        Value::Null,
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "profile@example.com");
    assert_eq!(body["name"], "Profile");
}

#[tokio::test]
async fn auth_error_shape_uses_common_code_and_message() {
    let app = TestApp::spawn().await;

    let (status, _, body) = json_request(
        app.router.clone(),
        Method::GET,
        "/api/me",
        Value::Null,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
    assert!(body["message"].as_str().unwrap().contains("authentication"));
}
