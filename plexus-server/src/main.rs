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

    let state = app::AppState::new_with_openai_runtime(pool, cfg, openai);
    plexus_server::chat::worker::spawn_pending_workers(state.clone())
        .await
        .map_err(|err| io::Error::other(err.message))?;

    let listener = tokio::net::TcpListener::bind(state.config().bind).await?;
    axum::serve(listener, app::router(state)).await?;
    Ok(())
}
