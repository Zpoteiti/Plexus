use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};

#[derive(Clone)]
#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
struct FakeState {
    model: String,
    api_key: String,
    mode: FakeMode,
    delay: Duration,
    chat_calls: Arc<AtomicUsize>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

#[derive(Clone)]
#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
enum FakeMode {
    Valid,
    MissingModel,
    MalformedModels,
    ImageUnsupportedThenValid,
    AlwaysUnavailable,
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
pub struct FakeOpenAi {
    pub base_url: String,
    handle: JoinHandle<()>,
    chat_calls: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
impl FakeOpenAi {
    pub async fn valid() -> Self {
        Self::spawn(FakeMode::Valid, Duration::ZERO).await
    }

    pub async fn missing_model() -> Self {
        Self::spawn(FakeMode::MissingModel, Duration::ZERO).await
    }

    pub async fn malformed_models() -> Self {
        Self::spawn(FakeMode::MalformedModels, Duration::ZERO).await
    }

    pub async fn delayed(delay: Duration) -> Self {
        Self::spawn(FakeMode::Valid, delay).await
    }

    pub async fn image_unsupported_then_valid() -> Self {
        Self::spawn(FakeMode::ImageUnsupportedThenValid, Duration::ZERO).await
    }

    pub async fn always_unavailable() -> Self {
        Self::spawn(FakeMode::AlwaysUnavailable, Duration::ZERO).await
    }

    pub fn model(&self) -> &'static str {
        "plexus-fake-qa"
    }

    pub fn api_key(&self) -> &'static str {
        "plexus-mock-key"
    }

    pub fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    pub fn chat_call_count(&self) -> usize {
        self.chat_calls.load(Ordering::SeqCst)
    }

    async fn spawn(mode: FakeMode, delay: Duration) -> Self {
        let chat_calls = Arc::new(AtomicUsize::new(0));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let state = FakeState {
            model: "plexus-fake-qa".to_string(),
            api_key: "plexus-mock-key".to_string(),
            mode,
            delay,
            chat_calls: chat_calls.clone(),
            in_flight,
            max_in_flight: max_in_flight.clone(),
        };

        let router = Router::new()
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(chat))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        Self {
            base_url: format!("http://{addr}/v1"),
            handle,
            chat_calls,
            max_in_flight,
        }
    }
}

impl Drop for FakeOpenAi {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
struct InFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
async fn models(State(state): State<FakeState>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(error("invalid_api_key")));
    }

    match state.mode {
        FakeMode::Valid => (
            StatusCode::OK,
            Json(json!({
                "object": "list",
                "data": [{"id": state.model, "object": "model"}]
            })),
        ),
        FakeMode::MissingModel => (
            StatusCode::OK,
            Json(json!({
                "object": "list",
                "data": [{"id": "different-model", "object": "model"}]
            })),
        ),
        FakeMode::MalformedModels => (StatusCode::OK, Json(json!({"data": "bad"}))),
        FakeMode::ImageUnsupportedThenValid | FakeMode::AlwaysUnavailable => (
            StatusCode::OK,
            Json(json!({
                "object": "list",
                "data": [{"id": state.model, "object": "model"}]
            })),
        ),
    }
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
async fn chat(
    State(state): State<FakeState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.chat_calls.fetch_add(1, Ordering::SeqCst);

    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(error("invalid_api_key")));
    }

    if body.get("model").and_then(Value::as_str) != Some(state.model.as_str()) {
        return (StatusCode::NOT_FOUND, Json(error("model_not_found")));
    }

    let current = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    let _in_flight = InFlightGuard {
        counter: state.in_flight.clone(),
    };
    update_max(&state.max_in_flight, current);
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }

    let stream = body.get("stream").and_then(Value::as_bool);
    if stream != Some(false) {
        return (StatusCode::BAD_REQUEST, Json(error("stream_must_be_false")));
    }

    if matches!(state.mode, FakeMode::AlwaysUnavailable) {
        return (
            StatusCode::from_u16(529).unwrap(),
            Json(error("overloaded")),
        );
    }

    if matches!(state.mode, FakeMode::ImageUnsupportedThenValid) && has_image_url(&body) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(error("image_unsupported")),
        );
    }

    let last_user = body["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message["role"] == "user")
        })
        .map(|message| content_to_text(&message["content"]))
        .unwrap_or_default();

    let content = if matches!(state.mode, FakeMode::ImageUnsupportedThenValid) {
        "image stripped fallback"
    } else {
        match last_user.as_str() {
            "hello" => "hi",
            "ping" => "pong",
            _ => "I do not have a fixture for that.",
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "id": "chatcmpl_test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop"
            }]
        })),
    )
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
fn authorized(state: &FakeState, headers: &HeaderMap) -> bool {
    let expected = format!("Bearer {}", state.api_key);
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some(expected.as_str())
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
fn error(code: &str) -> Value {
    json!({"error": {"message": code, "type": "invalid_request_error", "code": code}})
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
fn content_to_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
fn has_image_url(value: &Value) -> bool {
    value
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        blocks.iter().any(|block| {
                            block.get("type").and_then(Value::as_str) == Some("image_url")
                        })
                    })
            })
        })
}

#[allow(
    dead_code,
    reason = "compiled by shared test support; used by M1b OpenAI client tests"
)]
fn update_max(max: &AtomicUsize, current: usize) {
    let mut observed = max.load(Ordering::SeqCst);
    while current > observed {
        match max.compare_exchange(observed, current, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(next) => observed = next,
        }
    }
}
