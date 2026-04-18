/// Static file serving — from disk or embedded in the binary.

// ── Embedded mode ──────────────────────────────────────────────────
#[cfg(feature = "embed-frontend")]
mod inner {
    use axum::{
        Router,
        body::Body,
        http::{Request, Response, StatusCode, header},
        routing::get,
    };
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "../plexus-frontend/dist"]
    struct Asset;

    /// Serve frontend assets that were embedded at compile time.
    pub fn static_file_service() -> Router {
        Router::new().fallback(get(serve_embedded))
    }

    async fn serve_embedded(req: Request<Body>) -> Response<Body> {
        let path = req.uri().path().trim_start_matches('/');

        // Try the exact path, then fall back to index.html (SPA routing).
        let (data, mime) = match Asset::get(path) {
            Some(file) => {
                let mime = mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string();
                (file.data, mime)
            }
            None => match Asset::get("index.html") {
                Some(file) => (file.data, "text/html".to_string()),
                None => {
                    return Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Body::from("frontend not embedded"))
                        .unwrap();
                }
            },
        };

        Response::builder()
            .header(header::CONTENT_TYPE, mime)
            .body(Body::from(data.into_owned()))
            .unwrap()
    }
}

// ── Disk mode (default) ────────────────────────────────────────────
#[cfg(not(feature = "embed-frontend"))]
mod inner {
    use tower_http::services::{ServeDir, ServeFile};

    pub fn static_file_service(frontend_dir: &str) -> ServeDir<ServeFile> {
        let index = format!("{frontend_dir}/index.html");
        ServeDir::new(frontend_dir).fallback(ServeFile::new(index))
    }
}

pub use inner::*;
