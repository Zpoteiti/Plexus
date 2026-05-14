use crate::app::AppState;
use axum::{
    Router,
    routing::{get, post},
};

pub mod admin;
pub mod auth;
pub mod me;
pub mod sessions;
mod validation;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(me::get_me).patch(me::patch_me))
        .route(
            "/api/admin/config",
            get(admin::get_config).patch(admin::patch_config),
        )
        .route(
            "/api/sessions",
            get(sessions::list_sessions).post(sessions::create_session),
        )
        .route(
            "/api/sessions/{id}",
            get(sessions::get_session)
                .patch(sessions::rename_session)
                .delete(sessions::delete_session),
        )
        .route(
            "/api/sessions/{id}/messages",
            get(sessions::list_messages).post(sessions::post_message),
        )
        .route("/api/sessions/{id}/stream", get(sessions::stream_session))
}
