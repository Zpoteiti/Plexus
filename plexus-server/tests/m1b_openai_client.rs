mod support;

use plexus_common::LlmApiKey;
use plexus_server::openai::{
    ChatCompletionRequest, ChatMessage, ChatRole, OpenAiClient, OpenAiConfig, OpenAiRuntime,
};
use std::time::{Duration, Instant};
use support::fake_openai::FakeOpenAi;

fn config(fake: &FakeOpenAi) -> OpenAiConfig {
    OpenAiConfig {
        endpoint: fake.base_url.parse().unwrap(),
        api_key: LlmApiKey::new(fake.api_key().to_string()),
        model: fake.model().to_string(),
    }
}

#[tokio::test]
async fn validate_config_accepts_model_from_models_response() {
    let fake = FakeOpenAi::valid().await;
    OpenAiClient::new()
        .validate_config(&config(&fake))
        .await
        .expect("valid config");
}

#[tokio::test]
async fn validate_config_rejects_missing_model() {
    let fake = FakeOpenAi::missing_model().await;
    let err = OpenAiClient::new()
        .validate_config(&config(&fake))
        .await
        .expect_err("missing model should reject");
    assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn validate_config_rejects_unauthorized_models_response() {
    let fake = FakeOpenAi::valid().await;
    let mut cfg = config(&fake);
    cfg.api_key = LlmApiKey::new("wrong-key".to_string());
    let err = OpenAiClient::new()
        .validate_config(&cfg)
        .await
        .expect_err("bad key should reject");
    assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn validate_config_rejects_malformed_models_response() {
    let fake = FakeOpenAi::malformed_models().await;
    let err = OpenAiClient::new()
        .validate_config(&config(&fake))
        .await
        .expect_err("malformed response should reject");
    assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn chat_completion_sends_non_streaming_request_and_returns_content() {
    let fake = FakeOpenAi::valid().await;
    let response = OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
                max_tokens: None,
                temperature: None,
            },
        )
        .await
        .expect("chat response");

    assert_eq!(response.content, "hi");
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn runtime_concurrency_limit_caps_in_flight_chat_requests() {
    let fake = FakeOpenAi::delayed(Duration::from_millis(150)).await;
    let runtime = OpenAiRuntime::new(1).expect("valid runtime");
    let cfg = config(&fake);
    let request = ChatCompletionRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "ping".to_string(),
        }],
        max_tokens: None,
        temperature: None,
    };

    let started = Instant::now();
    let (left, right) = tokio::join!(
        runtime.chat_completion(&cfg, request.clone()),
        runtime.chat_completion(&cfg, request),
    );

    left.expect("left response");
    right.expect("right response");
    assert_eq!(fake.max_in_flight(), 1);
    assert!(started.elapsed() >= Duration::from_millis(250));
}
