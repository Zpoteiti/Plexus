mod support;

use plexus_server::db::users;
use support::TestApp;

#[tokio::test]
async fn create_user_persists_without_returning_password_hash() {
    let app = TestApp::spawn().await;
    let user = users::create_user(
        &app.pool,
        "alice@example.com",
        "hash-value",
        "Alice",
        false,
    )
    .await
    .unwrap();

    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.name, "Alice");
    assert!(!user.is_admin);

    let stored_hash: (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stored_hash.0, "hash-value");
}
