use crate::db::system_config;
use sqlx::PgPool;
use std::path::Path;

/// Create the per-user workspace tree and seed it from system_config defaults
/// (or shipped template files as fallback).
///
/// Idempotent: re-creates missing files, but does not overwrite existing ones.
///
/// Pass `pool: None` to skip the system_config lookup entirely and use the
/// shipped templates directly. This is useful for FS-only tests that should
/// not block on a Postgres connect timeout.
pub async fn initialize_user_workspace(
    pool: Option<&PgPool>,
    workspace_root: &Path,
    user_id: &str,
) -> std::io::Result<()> {
    let user_root = workspace_root.join(user_id);
    tokio::fs::create_dir_all(&user_root).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&user_root, std::fs::Permissions::from_mode(0o700)).await?;
    }

    // Seed SOUL.md / MEMORY.md / HEARTBEAT.md from system_config defaults
    // (falling back to shipped templates if the system_config key is unset).
    for (key, filename, fallback) in [
        (
            "default_soul",
            "SOUL.md",
            include_str!("../../templates/workspace/SOUL.md"),
        ),
        (
            "default_memory",
            "MEMORY.md",
            include_str!("../../templates/workspace/MEMORY.md"),
        ),
        (
            "default_heartbeat",
            "HEARTBEAT.md",
            include_str!("../../templates/workspace/HEARTBEAT.md"),
        ),
    ] {
        let target = user_root.join(filename);
        if target.exists() {
            continue; // idempotent
        }
        let content = match pool {
            Some(pool) => match system_config::get(pool, key).await {
                Ok(Some(v)) => v,
                Ok(None) => fallback.to_string(),
                Err(e) => {
                    tracing::warn!(error = %e, key, user_id, "system_config lookup failed, using shipped template");
                    fallback.to_string()
                }
            },
            None => fallback.to_string(),
        };
        tokio::fs::write(&target, content.as_bytes()).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).await?;
        }
    }

    // Recursively copy templates/skills/ into the user's skills/
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/skills");
    let dst = user_root.join("skills");
    copy_dir_recursive(&src, &dst).await?;

    // Create uploads/
    tokio::fs::create_dir_all(user_root.join("uploads")).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(
            user_root.join("uploads"),
            std::fs::Permissions::from_mode(0o700),
        ).await.ok();
    }

    // Register the dream system cron job (Plan D). Idempotent via
    // ensure_system_cron_job — re-running workspace init doesn't duplicate.
    if let Some(pool) = pool {
        let timezone = crate::db::users::get_timezone(pool, user_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, user_id, "dream registration: timezone lookup failed, using UTC");
                "UTC".into()
            });
        if let Err(e) = crate::db::cron::ensure_system_cron_job(
            pool,
            user_id,
            "dream",
            "0 */2 * * *",    // every 2 hours, top of the hour
            &timezone,
            "",                // message: unused — dream handler bypasses the normal cron path
            "gateway",        // channel: required by the schema CHECK; unused for dream
            "-",              // chat_id: required by the schema; unused for dream
            false,            // deliver=false → publish_final skips; dream is silent
        ).await {
            tracing::warn!(error = %e, user_id, "failed to register dream system cron job");
            // Non-fatal: workspace registration still succeeded; admin can
            // re-init to retry. Dream won't fire for this user until then.
        }
    }

    Ok(())
}

async fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            Box::pin(copy_dir_recursive(&from, &to)).await?;
        } else if file_type.is_file() {
            // Skip if target exists (idempotent — don't clobber user edits).
            if !to.exists() {
                tokio::fs::copy(&from, &to).await?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    tokio::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o600)).await?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_initialize_creates_tree() {
        let root = TempDir::new().unwrap();
        initialize_user_workspace(None, root.path(), "alice")
            .await
            .unwrap();

        let user_dir = root.path().join("alice");
        assert!(user_dir.join("SOUL.md").exists());
        assert!(user_dir.join("MEMORY.md").exists());
        assert!(user_dir.join("HEARTBEAT.md").exists());
        assert!(user_dir.join("skills/create_skill/SKILL.md").exists());
        assert!(user_dir.join("uploads").exists());

        // Verify template content is present (the section-header invariant)
        let memory = tokio::fs::read_to_string(user_dir.join("MEMORY.md"))
            .await
            .unwrap();
        assert!(memory.contains("## User Facts"));
    }

    #[tokio::test]
    async fn test_initialize_is_idempotent() {
        let root = TempDir::new().unwrap();
        initialize_user_workspace(None, root.path(), "bob")
            .await
            .unwrap();

        // Edit a file, then re-run initialize — edit should survive.
        let memory = root.path().join("bob/MEMORY.md");
        tokio::fs::write(&memory, b"user-edited content")
            .await
            .unwrap();

        initialize_user_workspace(None, root.path(), "bob")
            .await
            .unwrap();

        let after = tokio::fs::read_to_string(&memory).await.unwrap();
        assert_eq!(after, "user-edited content");
    }

    #[tokio::test]
    async fn test_initialize_without_pool_uses_templates() {
        let root = TempDir::new().unwrap();
        initialize_user_workspace(None, root.path(), "nopool")
            .await
            .unwrap();

        // Without a pool, fallback templates must be used — verify shipped content appears.
        let soul = tokio::fs::read_to_string(root.path().join("nopool/SOUL.md"))
            .await
            .unwrap();
        assert!(soul.contains("baseline soul"));
    }

    #[tokio::test]
    #[ignore]  // needs DATABASE_URL
    async fn test_initialize_registers_dream_cron_job() {
        let url = std::env::var("DATABASE_URL").expect("set DATABASE_URL");
        let pool = crate::db::init_db(&url).await;

        let user_id = format!("d9-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let user_email = format!("{user_id}@test.local");
        crate::db::users::create_user(
            &pool, &user_id, &user_email, "", false,
        ).await.unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        initialize_user_workspace(Some(&pool), tmp.path(), &user_id).await.unwrap();

        let jobs = crate::db::cron::list_by_user(&pool, &user_id).await.unwrap();
        let dream = jobs.iter().find(|j| j.name == "dream")
            .expect("dream cron job should be registered for new user");
        assert_eq!(dream.kind, crate::db::cron::SYSTEM_KIND);
        assert!(!dream.deliver, "dream should have deliver=false");
        assert!(dream.enabled, "dream should be enabled on creation");
        assert_eq!(dream.cron_expr.as_deref(), Some("0 */2 * * *"));

        // Second init: idempotent (no duplicate)
        initialize_user_workspace(Some(&pool), tmp.path(), &user_id).await.unwrap();
        let jobs2 = crate::db::cron::list_by_user(&pool, &user_id).await.unwrap();
        let dream_count = jobs2.iter().filter(|j| j.name == "dream").count();
        assert_eq!(dream_count, 1, "ensure_system_cron_job should be idempotent");

        // Cleanup
        sqlx::query("DELETE FROM cron_jobs WHERE user_id = $1").bind(&user_id).execute(&pool).await.ok();
        sqlx::query("DELETE FROM users WHERE user_id = $1").bind(&user_id).execute(&pool).await.ok();
    }

    /// Requires a running Postgres with DATABASE_URL set and the system_config
    /// table present.  Run with: cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn test_initialize_uses_system_config_override() {
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set to run this test");
        let pool = PgPool::connect(&database_url)
            .await
            .expect("failed to connect to DB");

        // Set a custom default_soul in system_config
        system_config::set(&pool, "default_soul", "custom admin soul")
            .await
            .unwrap();

        let root = TempDir::new().unwrap();
        initialize_user_workspace(Some(&pool), root.path(), "carol")
            .await
            .unwrap();

        let soul = tokio::fs::read_to_string(root.path().join("carol/SOUL.md"))
            .await
            .unwrap();
        assert_eq!(soul, "custom admin soul");
    }
}
