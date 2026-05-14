use crate::{chat::ChatRuntime, config::ServerConfig, openai::OpenAiRuntime, routes};
use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    pub pool: PgPool,
    pub config: ServerConfig,
    pub openai: OpenAiRuntime,
    pub chat: ChatRuntime,
    pub admin_config_lock: Mutex<()>,
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
                chat: ChatRuntime::default(),
                admin_config_lock: Mutex::new(()),
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

    pub fn chat(&self) -> &ChatRuntime {
        &self.inner.chat
    }

    pub fn admin_config_lock(&self) -> &Mutex<()> {
        &self.inner.admin_config_lock
    }
}

pub fn router(state: AppState) -> Router {
    routes::router().with_state(state)
}
