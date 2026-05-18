mod support;

use plexus_common::tools::schemas::{
    DELETE_FILE_SCHEMA, DELETE_FOLDER_SCHEMA, EDIT_FILE_SCHEMA, GLOB_SCHEMA, GREP_SCHEMA,
    LIST_DIR_SCHEMA, READ_FILE_SCHEMA, WRITE_FILE_SCHEMA,
};
use plexus_common::{Code, ErrorCode};
use plexus_server::tools::registry::{FileToolRegistry, SERVER_DEVICE, merged_file_tool_schemas};
use serde_json::json;

#[test]
fn shared_file_source_schemas_do_not_contain_plexus_device() {
    for schema in [
        &*READ_FILE_SCHEMA,
        &*WRITE_FILE_SCHEMA,
        &*EDIT_FILE_SCHEMA,
        &*DELETE_FILE_SCHEMA,
        &*DELETE_FOLDER_SCHEMA,
        &*LIST_DIR_SCHEMA,
        &*GLOB_SCHEMA,
        &*GREP_SCHEMA,
    ] {
        let props = schema["input_schema"]["properties"].as_object().unwrap();
        assert!(!props.contains_key("plexus_device"));
    }
}

#[test]
fn merge_v0_injects_required_server_device() {
    let schemas = merged_file_tool_schemas();
    let read_file = schemas
        .iter()
        .find(|schema| schema["name"] == "read_file")
        .unwrap();
    let input = &read_file["input_schema"];

    assert_eq!(
        input["properties"]["plexus_device"]["enum"],
        json!([SERVER_DEVICE])
    );
    assert_eq!(
        input["properties"]["plexus_device"]["x-plexus-device"],
        json!(true)
    );
    assert!(
        input["required"]
            .as_array()
            .unwrap()
            .contains(&json!("plexus_device"))
    );
}

async fn set_quota(app: &support::TestApp, quota: i64) {
    sqlx::query(
        "INSERT INTO system_config (key, value, updated_at)
         VALUES ('quota_bytes', $1, NOW())
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(json!(quota))
    .execute(&app.pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn file_tool_registry_rejects_missing_or_non_server_device() {
    let app = support::TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    let registry = FileToolRegistry::new(app.state.workspace_fs().clone());

    let err = registry
        .call(user_id, "read_file", json!({"path": "a.txt"}))
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidArgs);

    let err = registry
        .call(
            user_id,
            "read_file",
            json!({"plexus_device": "devbox", "path": "a.txt"}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidArgs);
}

#[tokio::test]
async fn write_read_edit_list_and_delete_file_tools_use_workspace_fs() {
    let app = support::TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;
    let registry = FileToolRegistry::new(app.state.workspace_fs().clone());

    registry
        .call(
            user_id,
            "write_file",
            json!({
                "plexus_device": "server",
                "path": "docs/a.txt",
                "content": "hello world"
            }),
        )
        .await
        .unwrap();

    let read = registry
        .call(
            user_id,
            "read_file",
            json!({"plexus_device": "server", "path": "docs/a.txt"}),
        )
        .await
        .unwrap();
    assert!(read.contains("hello world"));

    let edit = registry
        .call(
            user_id,
            "edit_file",
            json!({
                "plexus_device": "server",
                "path": "docs/a.txt",
                "old_text": "world",
                "new_text": "plexus"
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&edit).unwrap()["replacements"],
        1
    );

    let list = registry
        .call(
            user_id,
            "list_dir",
            json!({"plexus_device": "server", "path": "docs"}),
        )
        .await
        .unwrap();
    assert!(list.contains("a.txt"));

    let grep = registry
        .call(
            user_id,
            "grep",
            json!({"plexus_device": "server", "pattern": "plexus", "path": "docs"}),
        )
        .await
        .unwrap();
    assert!(grep.contains("hello plexus"));

    registry
        .call(
            user_id,
            "delete_file",
            json!({"plexus_device": "server", "path": "docs/a.txt"}),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn file_tool_edit_rejects_empty_old_text() {
    let app = support::TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;
    let registry = FileToolRegistry::new(app.state.workspace_fs().clone());

    registry
        .call(
            user_id,
            "write_file",
            json!({"plexus_device": "server", "path": "docs/a.txt", "content": "hello"}),
        )
        .await
        .unwrap();

    let err = registry
        .call(
            user_id,
            "edit_file",
            json!({
                "plexus_device": "server",
                "path": "docs/a.txt",
                "old_text": "",
                "new_text": "prefix"
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidArgs);
}
