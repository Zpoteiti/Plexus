use crate::error::ApiError;
use axum::http::StatusCode;
use plexus_common::{ErrorCode, LlmApiKey};
use reqwest::Url;
use serde::{Deserialize, Serialize};
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
        let url = endpoint_url(&cfg.endpoint, "chat/completions")?;
        let body = ChatRequestBody {
            model: &cfg.model,
            messages: &request.messages,
            stream: false,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
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
                    if attempt < 2 {
                        tokio::time::sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    break;
                }
            };

            let status = response.status();
            if !status.is_success() {
                last_error = Some(provider_http_error(format!(
                    "LLM chat request returned HTTP {status}"
                )));
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
            let content = choice
                .message
                .content
                .ok_or_else(|| provider_http_error("LLM chat response had no assistant content"))?;

            return Ok(ChatCompletionResponse {
                content,
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

fn retry_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_millis(100),
        1 => Duration::from_millis(250),
        _ => Duration::from_millis(500),
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ChatCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatCompletionResponse {
    pub content: String,
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
    messages: &'a [ChatMessage],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
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
}

fn invalid_provider_config(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidArgs, message)
}

fn provider_http_error(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, ErrorCode::HttpError, message)
}
