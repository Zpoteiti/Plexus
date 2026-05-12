use plexus_server::{app, config, db};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::ServerConfig::from_env()?;
    tokio::fs::create_dir_all(&cfg.workspace_root).await?;
    let pool = db::connect(&cfg.database_url).await?;
    db::bootstrap(&pool).await?;

    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    axum::serve(listener, app::router(app::AppState::new(pool, cfg))).await?;
    Ok(())
}
