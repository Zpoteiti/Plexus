use sqlx::{PgPool, postgres::PgPoolOptions};

pub mod devices;
pub mod messages;
pub mod pending_messages;
pub mod sessions;
pub mod system_config;
pub mod users;

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}

pub async fn bootstrap(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(pool)
        .await?;
    Ok(())
}
