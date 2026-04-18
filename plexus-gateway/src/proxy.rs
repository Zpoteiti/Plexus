use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tracing::warn;

const HOP_BY_HOP: &[&str] = &[
    "host", "connection", "transfer-encoding", "upgrade",
    "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer",
];

pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
) -> Response {
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();

    // Path traversal check
    if path.contains("..") {
        return (StatusCode::UNPROCESSABLE_ENTITY, "path traversal not allowed").into_response();
    }

    // JWT validation — skip for /api/auth/*
    let is_public = path.starts_with("/api/auth/");
    if !is_public {
        let auth_header = req.headers().get("authorization").and_then(|v| v.to_str().ok());
        match auth_header {
            Some(h) if h.starts_with("Bearer ") => {
                let token = &h[7..];
                if let Err(e) = crate::jwt::validate(token, &state.config.jwt_secret) {
                    warn!("proxy: JWT validation failed: {e}");
                    return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
                }
            }
            _ => {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            }
        }
    }

    // Build upstream URL
    let upstream = format!("{}{}{}", state.config.server_api_url, path, query);

    // Copy headers, strip hop-by-hop
    let method = req.method().clone();
    let mut upstream_headers = reqwest::header::HeaderMap::new();
    for (key, value) in req.headers() {
        let name = key.as_str().to_lowercase();
        if !HOP_BY_HOP.contains(&name.as_str()) {
            if let Ok(rname) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                    upstream_headers.insert(rname, val);
                }
            }
        }
    }

    let body_bytes = match axum::body::to_bytes(req.into_body(), state.config.upload_max_bytes).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response();
        }
    };

    let upstream_req = state
        .http_client
        .request(method, &upstream)
        .headers(upstream_headers)
        .body(body_bytes);

    let upstream_resp = match upstream_req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("proxy: upstream error: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                serde_json::json!({"error":{"code":"upstream_unreachable","message":e.to_string()}}).to_string(),
            ).into_response();
        }
    };

    let status = StatusCode::from_u16(upstream_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let resp_headers = upstream_resp.headers().clone();

    let resp_bytes = match upstream_resp.bytes().await {
        Ok(b) => {
            if b.len() > state.config.upload_max_bytes {
                return (
                    StatusCode::BAD_GATEWAY,
                    serde_json::json!({"error":{"code":"upstream_too_large","message":"upstream response body exceeded size limit"}}).to_string(),
                ).into_response();
            }
            b
        }
        Err(e) => {
            warn!("proxy: failed to read upstream response: {e}");
            return (StatusCode::BAD_GATEWAY, "upstream read error").into_response();
        }
    };

    let mut response = Response::builder().status(status);
    for (key, value) in &resp_headers {
        let name = key.as_str().to_lowercase();
        if !HOP_BY_HOP.contains(&name.as_str()) {
            response = response.header(key.as_str(), value.as_bytes());
        }
    }
    response.body(Body::from(resp_bytes)).unwrap()
}
