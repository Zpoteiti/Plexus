mod support;

use plexus_common::WorkspaceError;
use plexus_server::workspace::WorkspaceFs;
use serde_json::json;
use support::TestApp;

async fn set_quota(app: &TestApp, quota: i64) {
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
async fn write_read_and_delete_file_under_user_workspace() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    fs.write_file(user_id, "notes/hello.txt", b"hello".to_vec())
        .await
        .unwrap();

    let bytes = fs.read_file(user_id, "notes/hello.txt").await.unwrap();
    assert_eq!(bytes, b"hello");

    fs.delete_file(user_id, "notes/hello.txt").await.unwrap();
    let err = fs.read_file(user_id, "notes/hello.txt").await.unwrap_err();
    assert!(matches!(err, WorkspaceError::NotFound(_)));
}

#[tokio::test]
async fn absolute_write_path_inside_user_workspace_is_allowed() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let path = app
        .workspace_root
        .path()
        .join(user_id.to_string())
        .join("absolute/inside.txt");
    fs.write_file(user_id, path.to_str().unwrap(), b"inside".to_vec())
        .await
        .unwrap();

    let bytes = fs.read_file(user_id, "absolute/inside.txt").await.unwrap();
    assert_eq!(bytes, b"inside");
}

#[tokio::test]
async fn absolute_write_path_outside_user_workspace_is_rejected() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let outside = tempfile::tempdir().unwrap();
    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let err = fs
        .write_file(
            user_id,
            outside.path().join("outside.txt").to_str().unwrap(),
            b"outside".to_vec(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));
}

#[tokio::test]
async fn empty_write_path_is_rejected() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let err = fs
        .write_file(user_id, "", b"empty".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_parent_escape_does_not_create_external_parent() {
    use std::os::unix::fs::symlink;

    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let user_root = app.workspace_root.path().join(user_id.to_string());
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), user_root.join("escape")).unwrap();

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let err = fs
        .write_file(user_id, "escape/nested/file.txt", b"escape".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));
    assert!(!outside.path().join("nested").exists());
    assert!(!outside.path().join("nested/file.txt").exists());
}

#[tokio::test]
async fn path_traversal_is_rejected() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let err = fs
        .write_file(user_id, "../escape.txt", b"no".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));
}

#[tokio::test]
async fn missing_quota_blocks_writes() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let err = fs
        .write_file(user_id, "a.txt", b"a".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::QuotaNotConfigured));
}

#[tokio::test]
async fn upload_too_large_uses_single_op_cap() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 100).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let err = fs
        .write_file(user_id, "large.bin", vec![b'x'; 81])
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::UploadTooLarge { .. }));
}

#[tokio::test]
async fn soft_lock_blocks_writes_but_allows_delete() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 1_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    fs.write_file(user_id, "old.bin", vec![b'x'; 700])
        .await
        .unwrap();

    set_quota(&app, 100).await;
    let err = fs
        .write_file(user_id, "new.bin", b"no".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::SoftLocked));

    fs.delete_file(user_id, "old.bin").await.unwrap();
    let quota = fs.quota(user_id).await.unwrap();
    assert!(!quota.locked);
}
