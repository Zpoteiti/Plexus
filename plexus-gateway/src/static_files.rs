use tower_http::services::{ServeDir, ServeFile};

pub fn static_file_service(frontend_dir: &str) -> ServeDir<ServeFile> {
    let index = format!("{frontend_dir}/index.html");
    ServeDir::new(frontend_dir).fallback(ServeFile::new(index))
}
