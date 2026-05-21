mod support;

use axum::http::{Method, StatusCode};
use plexus_common::{protocol::WsFrame, version::PROTOCOL_VERSION};
use serde_json::json;
use std::time::Duration;
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
async fn protocol_mismatch_closes_4409() {
    let app = TestApp::spawn().await;
    let (jwt, _) = support::register_user(&app, "alice@example.com").await;
    let token = create_device(&app, &jwt).await;
    let base = app.spawn_server().await;

    let mut client = DeviceClient::connect(&base, Some(&token)).await;
    client.send_hello("999").await;
    assert_eq!(client.recv_close_code().await, 4409);
}
