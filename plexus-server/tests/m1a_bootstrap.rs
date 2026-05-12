mod support;

use support::TestApp;

#[tokio::test]
async fn bootstrap_creates_all_m1a_tables() {
    let app = TestApp::spawn().await;
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    let names: Vec<String> = rows.into_iter().map(|row| row.0).collect();

    for expected in [
        "cron_jobs",
        "devices",
        "discord_configs",
        "messages",
        "sessions",
        "system_config",
        "telegram_configs",
        "users",
        "workspace_members",
        "workspaces",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "missing table {expected}"
        );
    }
}

#[tokio::test]
async fn bootstrap_is_idempotent() {
    let app = TestApp::spawn().await;
    plexus_server::db::bootstrap(&app.pool).await.unwrap();
}
