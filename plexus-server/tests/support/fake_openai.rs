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
struct FakeState {
    model: String,
    api_key: String,
    mode: FakeMode,
    delay: Duration,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

#[derive(Clone)]
enum FakeMode {
    Valid,
    MissingModel,
    MalformedModels,
}

pub struct FakeOpenAi {
    pub base_url: String,
    handle: JoinHandle<()>,
    max_in_flight: Arc<AtomicUsize>,
}

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

    pub fn model(&self) -> &'static str {
        "plexus-fake-qa"
    }

    pub fn api_key(&self) -> &'static str {
        "plexus-mock-key"
    }

    pub fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    async fn spawn(mode: FakeMode, delay: Duration) -> Self {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let state = FakeState {
            model: "plexus-fake-qa".to_string(),
            api_key: "plexus-mock-key".to_string(),
            mode,
            delay,
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
            max_in_flight,
        }
    }
}

impl Drop for FakeOpenAi {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct InFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

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
    }
}

async fn chat(
    State(state): State<FakeState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
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

    let last_user = body["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message["role"] == "user")
        })
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();

    let content = match last_user {
        "hello" => "hi",
        "ping" => "pong",
        _ => "I do not have a fixture for that.",
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

fn authorized(state: &FakeState, headers: &HeaderMap) -> bool {
    let expected = format!("Bearer {}", state.api_key);
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some(expected.as_str())
}

fn error(code: &str) -> Value {
    json!({"error": {"message": code, "type": "invalid_request_error", "code": code}})
}

fn update_max(max: &AtomicUsize, current: usize) {
    let mut observed = max.load(Ordering::SeqCst);
    while current > observed {
        match max.compare_exchange(observed, current, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(next) => observed = next,
        }
    }
}
