mod support;

use plexus_common::ErrorCode;
use plexus_server::{
    chat::prompt,
    db::{sessions, system_config, users},
};
use serde_json::json;
use support::TestApp;

#[tokio::test]
async fn prompt_reads_optional_soul_and_memory() {
    let app = TestApp::spawn().await;
    let user = users::create_user(&app.pool, "alice@example.com", "hash", "Alice", false)
        .await
        .unwrap();
    let user_dir = app.workspace_root.path().join(user.id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::write(user_dir.join("SOUL.md"), "Be concise.")
        .await
        .unwrap();
    tokio::fs::write(user_dir.join("MEMORY.md"), "Alice likes trains.")
        .await
        .unwrap();
    let session = sessions::create_web_session(&app.pool, user.id, "New chat")
        .await
        .unwrap();

    let text = prompt::build_system_prompt(app.workspace_root.path(), &user, &session)
        .await
        .unwrap();
    assert!(text.contains("## SOUL"));
    assert!(text.contains("Be concise."));
    assert!(text.contains("## MEMORY"));
    assert!(text.contains("Alice likes trains."));
    assert!(text.contains("M1c has no tools available"));
}

#[tokio::test]
async fn stored_llm_config_requires_identity_values() {
    let app = TestApp::spawn().await;
    let err = system_config::current_llm_config(&app.pool)
        .await
        .expect_err("missing config should reject");
    assert_eq!(err.code, ErrorCode::InvalidArgs);

    let mut values = std::collections::BTreeMap::new();
    values.insert(
        "llm_endpoint".to_string(),
        json!("http://127.0.0.1:1234/v1"),
    );
    values.insert("llm_api_key".to_string(), json!("test-key"));
    values.insert("llm_model".to_string(), json!("test-model"));
    let mut tx = app.pool.begin().await.unwrap();
    system_config::set_many(&mut tx, &values).await.unwrap();
    tx.commit().await.unwrap();

    let cfg = system_config::current_llm_config(&app.pool)
        .await
        .expect("stored config");
    assert_eq!(cfg.model, "test-model");
}
