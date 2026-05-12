use crate::error::ApiError;
use axum::http::StatusCode;
use plexus_common::{ErrorCode, LlmApiKey};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

pub const REDACTED_LLM_API_KEY: &str = "<redacted>";
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
}

#[derive(Clone)]
pub struct OpenAiRuntime {
    client: OpenAiClient,
    limiter: Arc<RwLock<Option<Arc<Semaphore>>>>,
}

impl Default for OpenAiRuntime {
    fn default() -> Self {
        Self::new(0)
    }
}

impl OpenAiRuntime {
    pub fn new(limit: i64) -> Self {
        Self {
            client: OpenAiClient::new(),
            limiter: Arc::new(RwLock::new(limit_to_semaphore(limit))),
        }
    }

    pub fn client(&self) -> &OpenAiClient {
        &self.client
    }

    pub async fn set_concurrency_limit(&self, limit: i64) -> Result<(), ApiError> {
        if limit < 0 {
            return Err(ApiError::invalid_args(
                "llm_max_concurrent_requests must be zero or positive",
            ));
        }
        *self.limiter.write().await = limit_to_semaphore(limit);
        Ok(())
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

fn limit_to_semaphore(limit: i64) -> Option<Arc<Semaphore>> {
    if limit > 0 {
        Some(Arc::new(Semaphore::new(limit as usize)))
    } else {
        None
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
