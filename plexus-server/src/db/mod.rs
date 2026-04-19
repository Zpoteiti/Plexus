//! Database initialization and CRUD modules.
//! All queries via sqlx::query / sqlx::query_as (runtime unchecked).

pub mod cron;
pub mod devices;
pub mod discord;
pub mod messages;
pub mod sessions;
pub mod system_config;
pub mod telegram;
pub mod users;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tracing::info;

pub async fn init_db(database_url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(plexus_common::consts::DB_POOL_MAX_CONNECTIONS)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(database_url)
        .await
        .expect("Failed to connect to database");

    initialize(&pool)
        .await
        .expect("Failed to initialize database schema");
    info!("Database initialized");
    pool
}

/// Load the canonical schema and seed default system_config rows.
///
/// The entire schema lives in `schema.sql` (one `CREATE TABLE IF NOT EXISTS`
/// per table, every FK already declared `ON DELETE CASCADE`). No cascade
/// migrations, no `ALTER TABLE` at boot — fresh installs and re-runs converge
/// on the same shape via idempotent DDL.
pub async fn initialize(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(pool)
        .await?;
    system_config::seed_defaults_if_missing(pool).await?;
    Ok(())
}
