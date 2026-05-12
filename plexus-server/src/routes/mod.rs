use crate::app::AppState;
use axum::Router;

pub mod admin;
pub mod auth;
pub mod me;

pub fn router() -> Router<AppState> {
    Router::new()
}
