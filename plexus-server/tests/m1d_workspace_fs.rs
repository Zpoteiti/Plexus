mod support;

use plexus_common::WorkspaceError;
use plexus_server::workspace::{WorkspaceAttachmentImage, WorkspaceFs};
use serde_json::json;
use support::TestApp;

async fn set_quota(app: &TestApp, quota: i64) {
    set_quota_value(app, json!(quota)).await;
}

async fn set_quota_value(app: &TestApp, value: serde_json::Value) {
    sqlx::query(
        "INSERT INTO system_config (key, value, updated_at)
         VALUES ('quota_bytes', $1, NOW())
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(value)
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

#[cfg(unix)]
#[tokio::test]
async fn quota_ignores_workspace_symlinks_to_external_content() {
    use std::os::unix::fs::symlink;

    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 100).await;

    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("large.bin"), vec![b'x'; 1_000]).unwrap();
    let user_root = app.workspace_root.path().join(user_id.to_string());
    symlink(
        outside.path().join("large.bin"),
        user_root.join("linked.bin"),
    )
    .unwrap();

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let quota = fs.quota(user_id).await.unwrap();
    assert_eq!(quota.bytes_used, 0);
    assert!(!quota.locked);
}

#[cfg(unix)]
#[tokio::test]
async fn file_operations_reject_final_symlink_targets() {
    use std::os::unix::fs::symlink;

    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let user_root = app.workspace_root.path().join(user_id.to_string());
    std::fs::write(user_root.join("real.txt"), b"real").unwrap();
    symlink(user_root.join("real.txt"), user_root.join("link.txt")).unwrap();

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let err = fs.read_file(user_id, "link.txt").await.unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));

    let err = fs
        .write_file(user_id, "link.txt", b"overwrite".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));

    let err = fs.delete_file(user_id, "link.txt").await.unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));
    assert!(user_root.join("link.txt").exists());
    assert_eq!(std::fs::read(user_root.join("real.txt")).unwrap(), b"real");
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
async fn invalid_quota_values_block_writes() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());

    for value in [json!(0), json!(-1), json!("1000")] {
        set_quota_value(&app, value).await;
        let err = fs
            .write_file(user_id, "invalid-quota.txt", b"x".to_vec())
            .await
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::QuotaNotConfigured));
    }
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
async fn attachment_image_reader_returns_non_image_without_full_image_payload() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    fs.write_file(user_id, "docs/readme.txt", b"not an image".to_vec())
        .await
        .unwrap();

    let image = fs
        .read_attachment_image(user_id, "docs/readme.txt")
        .await
        .unwrap();

    assert!(matches!(image, WorkspaceAttachmentImage::NonImage));
}

#[tokio::test]
async fn upload_collection_limit_uses_single_op_cap() {
    let app = TestApp::spawn().await;
    set_quota(&app, 100).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let limit = fs.upload_collection_limit(64 * 1024 * 1024).await.unwrap();

    assert_eq!(limit.max_bytes, 80);
    assert_eq!(limit.quota_bytes, 100);
}

#[tokio::test]
async fn cumulative_quota_rejects_writes_that_would_exceed_quota() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 1_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    fs.write_file(user_id, "existing.bin", vec![b'x'; 600])
        .await
        .unwrap();
    fs.write_file(user_id, "allowed.bin", vec![b'x'; 300])
        .await
        .unwrap();

    let err = fs
        .write_file(user_id, "rejected.bin", vec![b'x'; 300])
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::SoftLocked));
}

#[tokio::test]
async fn overwrite_quota_accounting_subtracts_existing_target_size() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 1_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    fs.write_file(user_id, "existing.bin", vec![b'x'; 600])
        .await
        .unwrap();
    fs.write_file(user_id, "existing.bin", vec![b'x'; 300])
        .await
        .unwrap();

    let quota = fs.quota(user_id).await.unwrap();
    assert_eq!(quota.bytes_used, 300);
}

#[tokio::test]
async fn write_file_rejects_existing_directory_target() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 10_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    tokio::fs::create_dir_all(
        app.workspace_root
            .path()
            .join(user_id.to_string())
            .join("dir"),
    )
    .await
    .unwrap();

    let err = fs
        .write_file(user_id, "dir", b"not a dir".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::PathOutsideWorkspace(_)));
}

#[tokio::test]
async fn concurrent_writes_are_serialized_for_quota_accounting() {
    let app = TestApp::spawn().await;
    let (_, user_id) = support::register_user(&app, "alice@example.com").await;
    set_quota(&app, 1_000).await;

    let fs = WorkspaceFs::new(app.workspace_root.path().to_path_buf(), app.pool.clone());
    let mut handles = Vec::new();
    for idx in 0..4 {
        let fs = fs.clone();
        handles.push(tokio::spawn(async move {
            fs.write_file(user_id, &format!("concurrent/{idx}.bin"), vec![b'x'; 300])
                .await
        }));
    }

    let mut failures = 0;
    for handle in handles {
        if handle.await.unwrap().is_err() {
            failures += 1;
        }
    }

    let quota = fs.quota(user_id).await.unwrap();
    assert!(failures >= 1);
    assert!(quota.bytes_used <= quota.quota_bytes);
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
