use plexus_server::{app, config, db, openai::OpenAiRuntime};
use std::io;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::ServerConfig::from_env()?;
    tokio::fs::create_dir_all(&cfg.workspace_root).await?;
    let pool = db::connect(&cfg.database_url).await?;
    db::bootstrap(&pool).await?;
    let llm_limit = db::system_config::current_concurrency_limit(&pool).await?;
    let openai = OpenAiRuntime::new(llm_limit)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.message))?;

    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    axum::serve(
        listener,
        app::router(app::AppState::new_with_openai_runtime(pool, cfg, openai)),
    )
    .await?;
    Ok(())
}
