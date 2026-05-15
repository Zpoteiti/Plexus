#[path = "support/fake_openai.rs"]
mod fake_openai;

use fake_openai::FakeOpenAi;
use plexus_common::{ChatRole, ContentBlock, ImageUrlBlock, LlmApiKey, ReasoningEffort};
use plexus_server::openai::{
    ChatCompletionRequest, ChatMessage, OpenAiClient, OpenAiConfig, OpenAiRuntime,
};
use std::time::{Duration, Instant};

fn config(fake: &FakeOpenAi) -> OpenAiConfig {
    OpenAiConfig {
        endpoint: fake.base_url.parse().unwrap(),
        api_key: LlmApiKey::new(fake.api_key().to_string()),
        model: fake.model().to_string(),
    }
}

fn user_message(text: &str) -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content: vec![ContentBlock::text(text)],
        reasoning_content: None,
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
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::None),
            },
        )
        .await
        .expect("chat response");

    assert_eq!(response.content, "hi");
    assert_eq!(response.reasoning_content, None);
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn chat_completion_sends_reasoning_controls_for_none() {
    let fake = FakeOpenAi::valid().await;
    OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::None),
            },
        )
        .await
        .expect("chat response");

    let body = fake.last_chat_body();
    assert_eq!(body["reasoning_effort"], "none");
    assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
}

#[tokio::test]
async fn chat_completion_omits_reasoning_controls_when_unspecified() {
    let fake = FakeOpenAi::valid().await;
    OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: None,
            },
        )
        .await
        .expect("chat response");

    let body = fake.last_chat_body();
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("chat_template_kwargs").is_none());
}

#[tokio::test]
async fn chat_completion_sends_reasoning_controls_for_high() {
    let fake = FakeOpenAi::valid().await;
    OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::High),
            },
        )
        .await
        .expect("chat response");

    let body = fake.last_chat_body();
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
}

#[tokio::test]
async fn assistant_history_sends_empty_reasoning_content() {
    let fake = FakeOpenAi::valid().await;
    OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![ChatMessage {
                    role: ChatRole::Assistant,
                    content: vec![ContentBlock::text("previous answer")],
                    reasoning_content: None,
                }],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect("chat response");

    let body = fake.last_chat_body();
    assert_eq!(body["messages"][0]["content"], "previous answer");
    assert_eq!(body["messages"][0]["reasoning_content"], "");
}

#[tokio::test]
async fn assistant_history_sends_stored_reasoning_content() {
    let fake = FakeOpenAi::valid().await;
    OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![ChatMessage {
                    role: ChatRole::Assistant,
                    content: vec![ContentBlock::text("previous answer")],
                    reasoning_content: Some("stored reasoning".to_string()),
                }],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect("chat response");

    let body = fake.last_chat_body();
    assert_eq!(body["messages"][0]["content"], "previous answer");
    assert_eq!(body["messages"][0]["reasoning_content"], "stored reasoning");
}

#[tokio::test]
async fn chat_completion_normalizes_native_reasoning_content() {
    let fake = FakeOpenAi::reasoning_content().await;
    let response = OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect("chat response");

    assert_eq!(response.content, "visible answer");
    assert_eq!(
        response.reasoning_content.as_deref(),
        Some("native reasoning")
    );
}

#[tokio::test]
async fn chat_completion_extracts_leading_think_block() {
    let fake = FakeOpenAi::think_tagged_content().await;
    let response = OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect("chat response");

    assert_eq!(response.content, "visible answer");
    assert_eq!(response.reasoning_content.as_deref(), Some("tag reasoning"));
}

#[tokio::test]
async fn chat_completion_does_not_retry_non_transient_request_errors() {
    let cfg = OpenAiConfig {
        endpoint: "ftp://example.com/v1".parse().unwrap(),
        api_key: LlmApiKey::new("test-key".to_string()),
        model: "test-model".to_string(),
    };

    let err = tokio::time::timeout(
        Duration::from_millis(50),
        OpenAiClient::new().chat_completion(
            &cfg,
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        ),
    )
    .await
    .expect("non-transient request error should not wait for retry")
    .expect_err("invalid URL scheme should fail");

    assert_eq!(err.status, axum::http::StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn chat_completion_accepts_endpoint_with_trailing_slash() {
    let fake = FakeOpenAi::valid().await;
    let mut cfg = config(&fake);
    cfg.endpoint = format!("{}/", fake.base_url).parse().unwrap();
    let response = OpenAiClient::new()
        .chat_completion(
            &cfg,
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect("chat response");

    assert_eq!(response.content, "hi");
}

#[tokio::test]
async fn runtime_concurrency_limit_caps_in_flight_chat_requests() {
    let fake = FakeOpenAi::delayed(Duration::from_millis(150)).await;
    let runtime = OpenAiRuntime::new(1).expect("valid runtime");
    let cfg = config(&fake);
    let request = ChatCompletionRequest {
        messages: vec![user_message("ping")],
        max_tokens: None,
        temperature: None,
        reasoning_effort: Some(ReasoningEffort::Medium),
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

#[tokio::test]
async fn chat_completion_sends_content_arrays() {
    let fake = FakeOpenAi::valid().await;
    let response = OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect("chat response");
    assert_eq!(response.content, "hi");
}

#[tokio::test]
async fn image_payload_error_retries_with_images_stripped() {
    let fake = FakeOpenAi::image_unsupported_then_valid().await;
    let response = OpenAiClient::new()
        .chat_completion(
            &config(&fake),
            ChatCompletionRequest {
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: vec![
                        ContentBlock::text("what is this"),
                        ContentBlock::ImageUrl {
                            image_url: ImageUrlBlock {
                                url: "data:image/png;base64,aGVsbG8=".to_string(),
                            },
                        },
                    ],
                    reasoning_content: None,
                }],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect("stripped retry response");
    assert_eq!(response.content, "image stripped fallback");
    assert_eq!(fake.chat_call_count(), 2);
}

#[tokio::test]
async fn auth_error_does_not_retry_or_strip_images() {
    let fake = FakeOpenAi::valid().await;
    let mut cfg = config(&fake);
    cfg.api_key = LlmApiKey::new("wrong-key".to_string());
    let err = OpenAiClient::new()
        .chat_completion(
            &cfg,
            ChatCompletionRequest {
                messages: vec![user_message("hello")],
                max_tokens: None,
                temperature: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
            },
        )
        .await
        .expect_err("auth failure");
    assert_eq!(err.status, axum::http::StatusCode::BAD_GATEWAY);
    assert!(err.message.contains("HTTP 401"));
    assert_eq!(fake.chat_call_count(), 1);
}
