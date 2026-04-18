//! Skills API endpoints.
//!
//! GET /api/skills — list skills (served from SkillsCache).
//! POST /api/skills, POST /api/skills/install, DELETE /api/skills/{name}
//!   — return 410 Gone (mutations now done via write_file/delete_file server tools).

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use plexus_common::error::ApiError;
use std::path::Path as FsPath;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

async fn list_skills(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let ws_root = FsPath::new(&state.config.workspace_root);
    let skills = state.skills_cache.get_or_load(&c.sub, ws_root).await;
    let list: Vec<_> = skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "always_on": s.always_on,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "skills": list })))
}

async fn create_skill(
    _headers: HeaderMap,
    _state: State<Arc<AppState>>,
    _body: axum::body::Bytes,
) -> (StatusCode, &'static str) {
    (
        StatusCode::GONE,
        "Skill creation via API is removed. Use write_file to create skills/{name}/SKILL.md \
         or web_fetch + write_file to install from a URL.",
    )
}

async fn install_skill(
    _headers: HeaderMap,
    _state: State<Arc<AppState>>,
    _body: axum::body::Bytes,
) -> (StatusCode, &'static str) {
    (
        StatusCode::GONE,
        "Skill install via API is removed. Use web_fetch to fetch the SKILL.md and write_file \
         to save it to skills/{name}/SKILL.md.",
    )
}

async fn delete_skill_handler(
    _headers: HeaderMap,
    _state: State<Arc<AppState>>,
    _name: Path<String>,
) -> (StatusCode, &'static str) {
    (
        StatusCode::GONE,
        "Skill deletion via API is removed. Use delete_file on skills/{name}/ (recursive: true).",
    )
}

pub fn skills_api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/skills", get(list_skills).post(create_skill))
        .route("/api/skills/install", post(install_skill))
        .route("/api/skills/{name}", delete(delete_skill_handler))
}
