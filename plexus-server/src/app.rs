use crate::{config::ServerConfig, routes};
use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    pub pool: PgPool,
    pub config: ServerConfig,
}

impl AppState {
    pub fn new(pool: PgPool, config: ServerConfig) -> Self {
        Self {
            inner: Arc::new(AppStateInner { pool, config }),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    pub fn config(&self) -> &ServerConfig {
        &self.inner.config
    }
}

pub fn router(state: AppState) -> Router {
    routes::router().with_state(state)
}
