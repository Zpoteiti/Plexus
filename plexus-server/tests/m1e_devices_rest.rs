mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::TestApp;

#[tokio::test]
async fn device_lifecycle_returns_token_once_and_hint_afterwards() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, created) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "MacBook Pro"}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = created["token"].as_str().unwrap();
    assert!(token.starts_with("plexus_dev_"));
    assert_eq!(created["device"]["name"], "macbook-pro");
    assert_eq!(created["device"]["workspace_path"], "~/plexus/workspace");
    assert_eq!(created["device"]["fs_policy"], "sandbox");
    assert_eq!(created["device"]["shell_timeout_max"], 300);
    assert_eq!(created["device"]["ssrf_whitelist"], json!([]));
    assert_eq!(created["device"]["mcp_servers"], json!({}));

    let (status, list) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/devices",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(list[0].get("token").is_none());
    assert_eq!(
        list[0]["token_hint"],
        format!("plexus_dev_...{}", &token[token.len() - 4..])
    );
    assert_eq!(list[0]["online"], false);
}

#[tokio::test]
async fn create_rejects_bad_names_and_duplicate_same_user() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    for name in ["server", "办公室电脑", "bad/name"] {
        let (status, body) = support::json_request(
            app.router.clone(),
            Method::POST,
            "/api/devices",
            json!({"name": name}),
            Some(&jwt),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["code"], "invalid_args");
    }

    let (status, _) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "Lab PC 01"}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "lab-pc-01"}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "invalid_args");

    let (other_jwt, _) = support::register_user(&app, "bob@example.com").await;
    let (status, _) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "lab-pc-01"}),
        Some(&other_jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn patch_can_rename_and_update_config_without_changing_token() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, created) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "old laptop"}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = created["token"].as_str().unwrap().to_string();

    let (status, patched) = support::json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/devices/old-laptop/config",
        json!({
            "name": "New Laptop",
            "workspace_path": "/tmp/plexus-testing-path",
            "fs_policy": "unrestricted",
            "shell_timeout_max": 120,
            "ssrf_whitelist": ["10.0.0.5:8080"],
            "mcp_servers": {"minimax": {"command": ["npx", "minimax"]}}
        }),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(patched.get("token").is_none());
    assert_eq!(patched["name"], "new-laptop");
    assert_eq!(patched["workspace_path"], "/tmp/plexus-testing-path");
    assert_eq!(patched["fs_policy"], "unrestricted");
    assert_eq!(patched["shell_timeout_max"], 120);
    assert_eq!(patched["ssrf_whitelist"], json!(["10.0.0.5:8080"]));

    let row: (String,) = sqlx::query_as("SELECT token FROM devices WHERE name = 'new-laptop'")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(row.0, token);
}

#[tokio::test]
async fn mcp_env_values_are_redacted_in_rest_but_persisted() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let mcp_servers = json!({
        "minimax": {
            "command": ["npx", "minimax"],
            "env": {
                "MINIMAX_API_KEY": "secret-api-key",
                "MINIMAX_BASE_URL": "https://api.example.test"
            },
            "description": "Minimax tools",
            "enabled": ["tool-*"]
        }
    });

    let (status, created) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "secret laptop", "mcp_servers": mcp_servers}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        created["device"]["mcp_servers"]["minimax"]["env"]["MINIMAX_API_KEY"],
        "<redacted>"
    );
    assert_eq!(
        created["device"]["mcp_servers"]["minimax"]["env"]["MINIMAX_BASE_URL"],
        "<redacted>"
    );
    assert_eq!(
        created["device"]["mcp_servers"]["minimax"]["command"],
        json!(["npx", "minimax"])
    );
    assert_eq!(
        created["device"]["mcp_servers"]["minimax"]["description"],
        "Minimax tools"
    );
    assert_eq!(
        created["device"]["mcp_servers"]["minimax"]["enabled"],
        json!(["tool-*"])
    );

    let row: (serde_json::Value,) =
        sqlx::query_as("SELECT mcp_servers FROM devices WHERE name = 'secret-laptop'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(row.0["minimax"]["env"]["MINIMAX_API_KEY"], "secret-api-key");

    let (status, list) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/devices",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list[0]["mcp_servers"]["minimax"]["env"]["MINIMAX_API_KEY"],
        "<redacted>"
    );

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/devices/secret-laptop/config",
        json!({"mcp_servers": created["device"]["mcp_servers"].clone()}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_args");

    let row: (serde_json::Value,) =
        sqlx::query_as("SELECT mcp_servers FROM devices WHERE name = 'secret-laptop'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(row.0["minimax"]["env"]["MINIMAX_API_KEY"], "secret-api-key");
}

#[tokio::test]
async fn create_rejects_malformed_config_payloads() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "bad ssrf", "ssrf_whitelist": ["10.0.0.5:8080", 42]}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_args");

    let (status, body) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "bad mcp", "mcp_servers": {"minimax": {"env": {"TOKEN": "secret"}}}}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_args");
}

#[tokio::test]
async fn regenerate_preserves_config_and_delete_removes_device() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;

    let (status, created) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "devbox", "workspace_path": "/tmp/plexus-testing-path"}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let old_token = created["token"].as_str().unwrap().to_string();

    let (status, regenerated) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices/devbox/regenerate-token",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let new_token = regenerated["token"].as_str().unwrap();
    assert_ne!(new_token, old_token);
    assert_eq!(
        regenerated["device"]["workspace_path"],
        "/tmp/plexus-testing-path"
    );

    let (status, _) = support::empty_request(
        app.router.clone(),
        Method::DELETE,
        "/api/devices/devbox",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM devices")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}
