use crate::{config::ServerConfig, openai::OpenAiRuntime, routes};
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
    pub openai: OpenAiRuntime,
}

impl AppState {
    pub fn new(pool: PgPool, config: ServerConfig) -> Self {
        Self::new_with_openai_runtime(pool, config, OpenAiRuntime::default())
    }

    pub fn new_with_openai_runtime(
        pool: PgPool,
        config: ServerConfig,
        openai: OpenAiRuntime,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                pool,
                config,
                openai,
            }),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    pub fn config(&self) -> &ServerConfig {
        &self.inner.config
    }

    pub fn openai(&self) -> &OpenAiRuntime {
        &self.inner.openai
    }
}

pub fn router(state: AppState) -> Router {
    routes::router().with_state(state)
}
