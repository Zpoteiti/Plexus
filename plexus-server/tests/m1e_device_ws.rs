mod support;

use axum::http::{Method, StatusCode};
use plexus_common::{ErrorCode, protocol::WsFrame, version::PROTOCOL_VERSION};
use serde_json::json;
use std::time::{Duration, Instant};
use support::{TestApp, device_client::DeviceClient};

async fn create_device(app: &TestApp, jwt: &str) -> String {
    let (status, body) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({"name": "devbox"}),
        Some(jwt),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["token"].as_str().unwrap().to_string()
}

async fn wait_for_device_online(app: &TestApp, jwt: &str) -> serde_json::Value {
    for _ in 0..20 {
        let (status, list) = support::json_request(
            app.router.clone(),
            Method::GET,
            "/api/devices",
            json!({}),
            Some(jwt),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        if list[0]["online"] == true {
            return list;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (status, list) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/devices",
        json!({}),
        Some(jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    list
}

#[tokio::test]
async fn valid_hello_receives_hello_ack_and_device_is_online() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    let hello_id = client.send_hello(PROTOCOL_VERSION).await;
    let frame = client.recv_frame().await;
    match frame {
        WsFrame::HelloAck(ack) => {
            assert_eq!(ack.id, hello_id);
            assert_eq!(ack.device_name, "devbox");
            assert_eq!(ack.config.workspace_path, "~/plexus/workspace");
        }
        other => panic!("expected hello_ack, got {other:?}"),
    }

    let list = wait_for_device_online(&app, &jwt).await;
    assert_eq!(list[0]["online"], true);
}

#[tokio::test]
async fn missing_authorization_header_closes_4401() {
    let app = TestApp::spawn().await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, None).await;
    assert_eq!(client.recv_close_code().await, 4401);
}

#[tokio::test]
async fn query_token_without_header_closes_4401() {
    let app = TestApp::spawn().await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect_path(&base, "/ws/device?token=not-accepted", None).await;
    assert_eq!(client.recv_close_code().await, 4401);
}

#[tokio::test]
async fn token_lookup_error_closes_retryable_1013() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    app.pool.close().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    let (code, reason) = client.recv_close().await;
    assert_eq!(code, 1013);
    assert_eq!(reason, r#"{"code":"io_error"}"#);
}

#[tokio::test]
async fn regenerated_token_during_pending_hello_closes_4401() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (status, _) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices/devbox/regenerate-token",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    client.send_hello(PROTOCOL_VERSION).await;
    assert_eq!(client.recv_close_code().await, 4401);
}

#[tokio::test]
async fn protocol_mismatch_closes_4409() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    client.send_hello("999").await;
    assert_eq!(client.recv_close_code().await, 4409);
}

#[tokio::test]
async fn duplicate_connection_replaces_old_connection() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut first = DeviceClient::connect(&base, Some(&token)).await;
    first.send_hello(PROTOCOL_VERSION).await;
    assert!(matches!(first.recv_frame().await, WsFrame::HelloAck(_)));

    let mut second = DeviceClient::connect(&base, Some(&token)).await;
    second.send_hello(PROTOCOL_VERSION).await;
    assert!(matches!(second.recv_frame().await, WsFrame::HelloAck(_)));

    assert_eq!(first.recv_close_code().await, 1000);

    let (status, list) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/devices",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["online"], true);
}

#[tokio::test]
async fn patch_sends_live_config_update() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    client.send_hello(PROTOCOL_VERSION).await;
    assert!(matches!(client.recv_frame().await, WsFrame::HelloAck(_)));

    let (status, _) = support::json_request(
        app.router.clone(),
        Method::PATCH,
        "/api/devices/devbox/config",
        json!({"workspace_path": "/tmp/plexus-testing-path"}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    loop {
        match client.recv_frame().await {
            WsFrame::ConfigUpdate(update) => {
                assert_eq!(update.config.workspace_path, "/tmp/plexus-testing-path");
                break;
            }
            WsFrame::Ping(ping) => client.reply_pong(ping.id).await,
            other => panic!("expected config_update, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn regenerate_closes_active_old_token_connection_4401() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    client.send_hello(PROTOCOL_VERSION).await;
    assert!(matches!(client.recv_frame().await, WsFrame::HelloAck(_)));

    let (status, _) = support::json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices/devbox/regenerate-token",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(client.recv_close_code().await, 4401);
}

#[tokio::test]
async fn delete_closes_active_connection_4401() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    client.send_hello(PROTOCOL_VERSION).await;
    assert!(matches!(client.recv_frame().await, WsFrame::HelloAck(_)));

    let (status, _) = support::empty_request(
        app.router.clone(),
        Method::DELETE,
        "/api/devices/devbox",
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(client.recv_close_code().await, 4401);
}

#[tokio::test]
async fn server_driven_heartbeat_pings_and_missed_pongs_close_4408() {
    let app = TestApp::spawn().await;
    app.state
        .devices()
        .set_heartbeat_for_tests(Duration::from_millis(200), 2)
        .await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    client.send_hello(PROTOCOL_VERSION).await;
    assert!(matches!(client.recv_frame().await, WsFrame::HelloAck(_)));

    let started = Instant::now();
    let _first_ping = match client.recv_frame().await {
        WsFrame::Ping(ping) => ping,
        other => panic!("expected ping, got {other:?}"),
    };
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(client.recv_close_code().await, 4408);
    assert!(
        started.elapsed() < Duration::from_millis(550),
        "heartbeat close took {:?}",
        started.elapsed()
    );
    let (status, list) = support::json_request(
        app.router.clone(),
        Method::GET,
        "/api/devices",
        json!({}),
        Some(&jwt),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["online"], false);
}

#[tokio::test]
async fn binary_post_handshake_frame_returns_malformed_error() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    client.send_hello(PROTOCOL_VERSION).await;
    assert!(matches!(client.recv_frame().await, WsFrame::HelloAck(_)));

    client.send_binary(vec![0, 1, 2]).await;

    loop {
        match client.recv_frame().await {
            WsFrame::Error(error) => {
                assert_eq!(error.code, ErrorCode::MalformedFrame);
                assert!(error.message.contains("binary"));
                break;
            }
            WsFrame::Ping(ping) => client.reply_pong(ping.id).await,
            other => panic!("expected error, got {other:?}"),
        }
    }
}
