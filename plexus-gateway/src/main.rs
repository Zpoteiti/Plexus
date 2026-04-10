mod config;
mod jwt;
mod proxy;
mod routing;
mod state;
mod static_files;
mod ws;

use crate::config::Config;
use crate::state::AppState;
use axum::{extract::State, routing::{any, get}, Json, Router};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;
use tracing_subscriber::EnvFilter;

async fn healthz(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let plexus_connected = state.plexus.read().await.is_some();
    let browsers = state.browsers.len();
    Json(serde_json::json!({
        "status": "ok",
        "plexus_connected": plexus_connected,
        "browsers": browsers,
    }))
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();
    let port = config.port;
    let frontend_dir = config.frontend_dir.clone();

    let state = Arc::new(AppState {
        config,
        browsers: Arc::new(DashMap::new()),
        plexus: Arc::new(RwLock::new(None)),
        http_client: reqwest::Client::new(),
        shutdown: CancellationToken::new(),
    });

    let static_service = static_files::static_file_service(&frontend_dir);

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/ws/chat", get(ws::chat::ws_chat))
        .route("/ws/plexus", get(ws::plexus::ws_plexus))
        .route("/api/{*rest}", any(proxy::proxy_handler))
        .fallback_service(static_service)
        .layer(RequestBodyLimitLayer::new(25 * 1024 * 1024))
        .with_state(state.clone());

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("Gateway listening on {}", listener.local_addr().unwrap());

    // Graceful shutdown
    let shutdown_state = state.clone();
    let shutdown_future = async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received");
        shutdown_state.shutdown.cancel();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_future)
        .await
        .unwrap();

    tracing::info!("Gateway shut down");
}
