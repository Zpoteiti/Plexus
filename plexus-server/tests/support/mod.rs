use plexus_common::{AdminToken, JwtSecret};
use plexus_server::{app, config::ServerConfig, db};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use std::{env, path::PathBuf};
use tempfile::TempDir;
use url::Url;
use uuid::Uuid;

#[allow(dead_code)]
pub struct TestApp {
    pub router: axum::Router,
    pub state: app::AppState,
    pub pool: PgPool,
    pub db_name: String,
    pub admin_url: String,
    pub workspace_root: TempDir,
}

impl TestApp {
    pub async fn spawn() -> Self {
        let admin_url = env::var("PLEXUS_TEST_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://plexus:plexus@127.0.0.1:5432/plexus".to_string());

        let db_name = format!("plexus_test_{}", Uuid::now_v7().simple());
        create_database(&admin_url, &db_name).await;
        let database_url = database_url_for_db(&admin_url, &db_name);
        let pool = db::connect(&database_url).await.expect("connect test db");
        let workspace_root = tempfile::tempdir().expect("temp workspace root");

        let cfg = ServerConfig {
            database_url,
            workspace_root: workspace_root.path().to_path_buf(),
            bind: "127.0.0.1:0".parse().unwrap(),
            jwt_secret: JwtSecret::new("test-jwt-secret-with-enough-entropy".to_string()),
            admin_token: Some(AdminToken::new("test-admin-token".to_string())),
            cookie_secure: false,
        };

        tokio::fs::create_dir_all(&cfg.workspace_root)
            .await
            .expect("create workspace root");
        db::bootstrap(&pool).await.expect("bootstrap test db");
        let state = app::AppState::new(pool.clone(), cfg);
        let router = app::router(state.clone());

        Self {
            router,
            state,
            pool,
            db_name,
            admin_url,
            workspace_root,
        }
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let admin_url = self.admin_url.clone();
        let db_name = self.db_name.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("drop runtime");
            rt.block_on(async move {
                drop_database(&admin_url, &db_name).await;
            });
        })
        .join()
        .expect("drop database thread");
    }
}

async fn create_database(admin_url: &str, db_name: &str) {
    let mut conn = PgConnection::connect(admin_url)
        .await
        .expect("connect admin database");
    let sql = format!(r#"CREATE DATABASE "{}""#, db_name);
    conn.execute(sql.as_str())
        .await
        .expect("create test database");
}

async fn drop_database(admin_url: &str, db_name: &str) {
    let mut conn = PgConnection::connect(admin_url)
        .await
        .expect("connect admin database for cleanup");
    let terminate = format!(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
        db_name
    );
    let _ = conn.execute(terminate.as_str()).await;
    let sql = format!(r#"DROP DATABASE IF EXISTS "{}""#, db_name);
    let _ = conn.execute(sql.as_str()).await;
}

fn database_url_for_db(admin_url: &str, db_name: &str) -> String {
    let mut url = Url::parse(admin_url).expect("valid postgres URL");
    url.set_path(&format!("/{db_name}"));
    url.to_string()
}

#[allow(dead_code)]
pub fn workspace_path(root: &TempDir, user_id: Uuid) -> PathBuf {
    root.path().join(user_id.to_string())
}

pub mod fake_openai;
