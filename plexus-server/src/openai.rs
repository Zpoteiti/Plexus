use crate::error::ApiError;
use axum::http::StatusCode;
use plexus_common::{
    ChatRole, ContentBlock, ErrorCode, LlmApiKey, ReasoningEffort, contains_image, strip_images,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

pub const REDACTED_LLM_API_KEY: &str = "<redacted>";
const MAX_CONCURRENCY_LIMIT: i64 = 1_000_000;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct OpenAiConfig {
    pub endpoint: Url,
    pub api_key: LlmApiKey,
    pub model: String,
}

#[derive(Clone)]
pub struct OpenAiClient {
    http: reqwest::Client,
}

impl Default for OpenAiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    pub async fn validate_config(&self, cfg: &OpenAiConfig) -> Result<(), ApiError> {
        let url = endpoint_url(&cfg.endpoint, "models")?;
        let response = self
            .http
            .get(url)
            .bearer_auth(cfg.api_key.expose_secret())
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|err| invalid_provider_config(format!("LLM models request failed: {err}")))?;

        if !response.status().is_success() {
            return Err(invalid_provider_config(format!(
                "LLM models request returned HTTP {}",
                response.status()
            )));
        }

        let models = response.json::<ModelsResponse>().await.map_err(|err| {
            invalid_provider_config(format!("LLM models response was malformed: {err}"))
        })?;

        if models.data.iter().any(|model| model.id == cfg.model) {
            Ok(())
        } else {
            Err(invalid_provider_config(format!(
                "LLM model '{}' was not listed by provider",
                cfg.model
            )))
        }
    }

    pub async fn chat_completion(
        &self,
        cfg: &OpenAiConfig,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, ApiError> {
        let has_images = request
            .messages
            .iter()
            .any(|message| contains_image(&message.content));

        match self.chat_completion_attempts(cfg, request.clone()).await {
            Ok(response) => Ok(response),
            Err(err) if has_images && err.message.contains("image-compatible retry") => {
                let stripped = ChatCompletionRequest {
                    messages: request
                        .messages
                        .into_iter()
                        .map(|message| ChatMessage {
                            role: message.role,
                            content: strip_images(&message.content),
                            reasoning_content: message.reasoning_content,
                        })
                        .collect(),
                    max_tokens: request.max_tokens,
                    temperature: request.temperature,
                    reasoning_effort: request.reasoning_effort,
                };
                self.chat_completion_attempts(cfg, stripped).await
            }
            Err(err) => Err(err),
        }
    }

    async fn chat_completion_attempts(
        &self,
        cfg: &OpenAiConfig,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, ApiError> {
        let url = endpoint_url(&cfg.endpoint, "chat/completions")?;
        let messages = provider_messages(&request.messages);
        let body = ChatRequestBody {
            model: &cfg.model,
            messages,
            stream: false,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            reasoning_effort: request.reasoning_effort,
            chat_template_kwargs: ChatTemplateKwargs {
                enable_thinking: request.reasoning_effort.enables_thinking(),
            },
        };

        let mut last_error: Option<ApiError> = None;
        for attempt in 0..3 {
            let result = self
                .http
                .post(url.clone())
                .bearer_auth(cfg.api_key.expose_secret())
                .timeout(DEFAULT_REQUEST_TIMEOUT)
                .json(&body)
                .send()
                .await;

            let response = match result {
                Ok(response) => response,
                Err(err) => {
                    last_error = Some(provider_http_error(format!(
                        "LLM chat request failed: {err}"
                    )));
                    if attempt < 2 && is_transient_request_error(&err) {
                        tokio::time::sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    break;
                }
            };

            let status = response.status();
            if !status.is_success() {
                let body_text = response.text().await.unwrap_or_default();
                let image_compatible = !is_auth_or_config_status(status)
                    && (is_image_compatibility_status(status)
                        || provider_error_mentions_image(&body_text));
                last_error = Some(provider_status_error(status, image_compatible));
                if attempt < 2 && is_transient_status(status) {
                    tokio::time::sleep(retry_delay(attempt)).await;
                    continue;
                }
                break;
            }

            let parsed = response.json::<ChatResponseBody>().await.map_err(|err| {
                provider_http_error(format!("LLM chat response was malformed: {err}"))
            })?;
            let choice = parsed
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| provider_http_error("LLM chat response had no choices"))?;
            let (content, reasoning_content) = normalize_response_message(choice.message)?;

            return Ok(ChatCompletionResponse {
                content,
                reasoning_content,
                finish_reason: choice.finish_reason,
            });
        }

        Err(last_error.unwrap_or_else(|| provider_http_error("LLM chat request failed")))
    }
}

#[derive(Clone)]
pub struct OpenAiRuntime {
    client: OpenAiClient,
    limiter: Arc<RwLock<Option<Arc<Semaphore>>>>,
}

impl Default for OpenAiRuntime {
    fn default() -> Self {
        Self::new_with_limiter(None)
    }
}

impl OpenAiRuntime {
    pub fn new(limit: i64) -> Result<Self, ApiError> {
        Ok(Self::new_with_limiter(limit_to_semaphore(limit)?))
    }

    fn new_with_limiter(limiter: Option<Arc<Semaphore>>) -> Self {
        Self {
            client: OpenAiClient::new(),
            limiter: Arc::new(RwLock::new(limiter)),
        }
    }

    pub fn client(&self) -> &OpenAiClient {
        &self.client
    }

    pub async fn set_concurrency_limit(&self, limit: i64) -> Result<(), ApiError> {
        // New cap applies to new acquisitions; in-flight permits keep the previous semaphore.
        *self.limiter.write().await = limit_to_semaphore(limit)?;
        Ok(())
    }

    pub async fn chat_completion(
        &self,
        cfg: &OpenAiConfig,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, ApiError> {
        let _permit = self.acquire_permit().await?;
        self.client.chat_completion(cfg, request).await
    }

    async fn acquire_permit(&self) -> Result<Option<OwnedSemaphorePermit>, ApiError> {
        let limiter = self.limiter.read().await.clone();
        match limiter {
            Some(semaphore) => semaphore.acquire_owned().await.map(Some).map_err(|_| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::HttpError,
                    "LLM concurrency limiter is closed",
                )
            }),
            None => Ok(None),
        }
    }
}

fn validate_concurrency_limit(limit: i64) -> Result<(), ApiError> {
    if limit < 0 {
        return Err(ApiError::invalid_args(
            "llm_max_concurrent_requests must be zero or positive",
        ));
    }
    if limit > MAX_CONCURRENCY_LIMIT {
        return Err(ApiError::invalid_args(format!(
            "llm_max_concurrent_requests must be at most {MAX_CONCURRENCY_LIMIT}"
        )));
    }
    Ok(())
}

fn limit_to_semaphore(limit: i64) -> Result<Option<Arc<Semaphore>>, ApiError> {
    validate_concurrency_limit(limit)?;
    Ok(if limit > 0 {
        Some(Arc::new(Semaphore::new(limit as usize)))
    } else {
        None
    })
}

fn endpoint_url(endpoint: &Url, suffix: &str) -> Result<Url, ApiError> {
    let mut url = endpoint.clone();
    let base = url.path().trim_end_matches('/');
    url.set_path(&format!("{base}/{suffix}"));
    Ok(url)
}

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn is_transient_request_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect()
}

fn is_auth_or_config_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::NOT_FOUND
}

fn is_image_compatibility_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 400 | 413 | 415 | 422)
}

fn provider_error_mentions_image(body_text: &str) -> bool {
    let lower = body_text.to_ascii_lowercase();
    lower.contains("image")
        || lower.contains("vision")
        || lower.contains("content block")
        || lower.contains("multimodal")
}

fn provider_status_error(status: reqwest::StatusCode, image_compatible: bool) -> ApiError {
    if image_compatible {
        provider_http_error(format!(
            "LLM chat request returned HTTP {status}; image-compatible retry"
        ))
    } else {
        provider_http_error(format!("LLM chat request returned HTTP {status}"))
    }
}

fn retry_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_millis(100),
        1 => Duration::from_millis(250),
        _ => Duration::from_millis(500),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Vec<ContentBlock>,
    pub reasoning_content: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ChatCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub reasoning_effort: ReasoningEffort,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatCompletionResponse {
    pub content: String,
    pub reasoning_content: Option<String>,
    pub finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    id: String,
}

#[derive(Serialize)]
struct ChatRequestBody<'a> {
    model: &'a str,
    messages: Vec<ProviderChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    reasoning_effort: ReasoningEffort,
    chat_template_kwargs: ChatTemplateKwargs,
}

#[derive(Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
}

#[derive(Serialize)]
struct ProviderChatMessage {
    role: ChatRole,
    content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponseBody {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
    reasoning_content: Option<String>,
}

fn provider_messages(messages: &[ChatMessage]) -> Vec<ProviderChatMessage> {
    messages
        .iter()
        .map(|message| {
            let is_assistant = message.role == ChatRole::Assistant;
            ProviderChatMessage {
                role: message.role,
                content: if is_assistant {
                    Value::String(content_blocks_to_text(&message.content))
                } else {
                    serde_json::to_value(&message.content).expect("content blocks serialize")
                },
                reasoning_content: is_assistant
                    .then(|| message.reasoning_content.clone().unwrap_or_default()),
            }
        })
        .collect()
}

fn content_blocks_to_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => text.clone(),
            other => serde_json::to_string(other).expect("content block serializes"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_response_message(
    message: ChatResponseMessage,
) -> Result<(String, Option<String>), ApiError> {
    let content = message
        .content
        .ok_or_else(|| provider_http_error("LLM chat response had no assistant content"))?;
    let (tag_reasoning, stripped_content) = extract_leading_think_block(&content);
    let reasoning_content = non_empty(message.reasoning_content).or(tag_reasoning);
    let content = if reasoning_content.is_some() {
        stripped_content
    } else {
        content
    };
    Ok((content, reasoning_content))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn extract_leading_think_block(content: &str) -> (Option<String>, String) {
    let trimmed = content.trim_start();
    let Some(after_open) = trimmed.strip_prefix("<think>") else {
        return (None, content.to_string());
    };
    let Some(close_index) = after_open.find("</think>") else {
        return (None, content.to_string());
    };
    let reasoning = after_open[..close_index].trim();
    let rest = after_open[close_index + "</think>".len()..]
        .trim_start()
        .to_string();
    if reasoning.is_empty() {
        (None, rest)
    } else {
        (Some(reasoning.to_string()), rest)
    }
}

fn invalid_provider_config(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidArgs, message)
}

fn provider_http_error(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, ErrorCode::HttpError, message)
}
