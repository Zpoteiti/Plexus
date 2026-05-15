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
        "pending_messages",
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
async fn bootstrap_applies_pending_messages_shape() {
    let app = TestApp::spawn().await;

    for column in [
        "id",
        "session_id",
        "user_id",
        "session_key",
        "content",
        "reasoning_effort",
        "received_at",
    ] {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT column_name FROM information_schema.columns
             WHERE table_name = 'pending_messages' AND column_name = $1",
        )
        .bind(column)
        .fetch_optional(&app.pool)
        .await
        .unwrap();
        assert_eq!(row.unwrap().0, column);
    }

    for index in [
        "idx_pending_messages_session_received",
        "idx_pending_messages_session_key_received",
    ] {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT indexname FROM pg_indexes
             WHERE tablename = 'pending_messages' AND indexname = $1",
        )
        .bind(index)
        .fetch_optional(&app.pool)
        .await
        .unwrap();
        assert_eq!(row.unwrap().0, index);
    }
}

#[tokio::test]
async fn bootstrap_is_idempotent() {
    let app = TestApp::spawn().await;
    plexus_server::db::bootstrap(&app.pool).await.unwrap();
}

#[tokio::test]
async fn bootstrap_does_not_seed_system_config_defaults() {
    let app = TestApp::spawn().await;
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM system_config")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
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
