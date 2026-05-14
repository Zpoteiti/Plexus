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

#[tokio::test]
async fn bootstrap_applies_m1c_session_shape() {
    let app = TestApp::spawn().await;

    let title: Option<(String,)> = sqlx::query_as(
        "SELECT column_name FROM information_schema.columns
         WHERE table_name = 'sessions' AND column_name = 'title'",
    )
    .fetch_optional(&app.pool)
    .await
    .unwrap();
    assert_eq!(title.unwrap().0, "title");

    let old_constraint: Option<(String,)> = sqlx::query_as(
        "SELECT constraint_name FROM information_schema.table_constraints
         WHERE table_name = 'sessions' AND constraint_name = 'sessions_session_key_key'",
    )
    .fetch_optional(&app.pool)
    .await
    .unwrap();
    assert!(old_constraint.is_none());

    let index: Option<(String,)> = sqlx::query_as(
        "SELECT indexname FROM pg_indexes
         WHERE tablename = 'sessions' AND indexname = 'idx_sessions_user_session_key'",
    )
    .fetch_optional(&app.pool)
    .await
    .unwrap();
    assert_eq!(index.unwrap().0, "idx_sessions_user_session_key");
}
