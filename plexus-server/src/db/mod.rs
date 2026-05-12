use sqlx::{postgres::PgPoolOptions, PgPool};

pub mod system_config;
pub mod users;

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}

pub async fn bootstrap(pool: &PgPool) -> Result<(), sqlx::Error> {
    let _ = pool;
    Ok(())
}
