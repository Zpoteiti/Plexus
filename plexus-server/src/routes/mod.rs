use crate::app::AppState;
use axum::{
    Router,
    routing::{get, post},
};

pub mod admin;
pub mod auth;
pub mod me;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(me::get_me).patch(me::patch_me))
}
