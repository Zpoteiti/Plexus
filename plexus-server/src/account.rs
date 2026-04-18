//! Account deletion orchestration.
//!
//! A single `delete_user_everywhere` entry point owns the teardown
//! sequence: stop channel bots, kick browser WebSockets, evict in-memory
//! state, wipe the workspace tree, then run the DB delete (which
//! cascades through every dependent table via the FKs added in AD-1).
//!
//! Each step is idempotent so a crash mid-delete doesn't leave a half-
//! deleted account. Errors on earlier steps do NOT abort the sequence —
//! we always try the DB delete, because "filesystem gone, DB row still
//! present" is worse than both being gone.

use crate::state::AppState;
use std::sync::Arc;
use tracing::{info, warn};

/// Summary of which teardown steps succeeded for observability.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DeletionSummary {
    pub discord_stopped: bool,
    pub telegram_stopped: bool,
    pub browsers_kicked: bool,
    pub in_memory_evicted: bool,
    pub files_wiped: bool,
    pub db_deleted: bool,
}

/// Delete a user everywhere: bots, browsers, in-memory state, files, DB.
/// See module doc for order + invariants. Returns a summary for caller logging.
pub async fn delete_user_everywhere(state: &Arc<AppState>, user_id: &str) -> DeletionSummary {
    let mut summary = DeletionSummary::default();
    info!(user_id, "Deleting user — starting teardown");

    // 1. Stop channel bots. Each is idempotent (no-op on a stopped bot).
    crate::channels::discord::stop_bot(user_id).await;
    summary.discord_stopped = true;

    crate::channels::telegram::stop_bot(user_id).await;
    summary.telegram_stopped = true;

    // 2. Kick live browser connections via the gateway.
    crate::channels::gateway::kick_user(state, user_id).await;
    summary.browsers_kicked = true;

    // 3. Evict in-memory state.
    evict_in_memory(state, user_id);
    summary.in_memory_evicted = true;

    // 4. Wipe workspace tree + quota cache.
    wipe_workspace(state, user_id).await;
    summary.files_wiped = true;

    // 5. DB delete — cascades through every dependent table.
    match crate::db::users::delete_user(&state.db, user_id).await {
        Ok(true) => {
            summary.db_deleted = true;
            info!(user_id, summary = ?summary, "user deleted");
        }
        Ok(false) => {
            warn!(user_id, "user already gone from DB (race?)");
        }
        Err(e) => {
            warn!(user_id, error = %e, "DB delete failed");
        }
    }

    summary
}

fn evict_in_memory(state: &Arc<AppState>, user_id: &str) {
    // Session handles: drop every session whose handle.user_id matches.
    state.sessions.retain(|_, handle| handle.user_id != user_id);

    // Devices: DashMap iteration + mutation is discouraged, collect keys first.
    let device_keys: Vec<String> = state
        .devices
        .iter()
        .filter(|entry| entry.value().user_id == user_id)
        .map(|entry| entry.key().clone())
        .collect();
    for key in &device_keys {
        state.devices.remove(key);
        state.pending.remove(key);
    }
    state.devices_by_user.remove(user_id);
    state.rate_limiter.remove(user_id);
    state.tool_schema_cache.remove(user_id);
    state.skills_cache.invalidate(user_id);
}

async fn wipe_workspace(state: &Arc<AppState>, user_id: &str) {
    let path = std::path::Path::new(&state.config.workspace_root).join(user_id);
    match tokio::fs::remove_dir_all(&path).await {
        Ok(()) => info!(user_id, "workspace wiped"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // User may never have been initialized (e.g., registration crashed
            // before writing any files). Benign.
        }
        Err(e) => warn!(user_id, error = %e, "failed to wipe workspace"),
    }
    state.quota.forget_user(user_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_wipe_workspace_removes_user_dir_and_quota_entry() {
        let tmp = TempDir::new().unwrap();
        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);

        // Seed the user's workspace dir with some content + a quota reservation.
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        tokio::fs::write(user_root.join("MEMORY.md"), b"secrets")
            .await
            .unwrap();
        state.quota.reserve_for_test("alice", 7);
        assert_eq!(state.quota.current_usage("alice"), 7);

        wipe_workspace(&state, "alice").await;

        assert!(!user_root.exists(), "user dir should be gone");
        assert_eq!(
            state.quota.current_usage("alice"),
            0,
            "quota cache entry should be forgotten"
        );
    }

    #[tokio::test]
    async fn test_wipe_workspace_handles_missing_dir() {
        let tmp = TempDir::new().unwrap();
        let state = crate::state::AppState::test_minimal(tmp.path());

        // User never had a workspace — this should NOT panic or error.
        wipe_workspace(&state, "ghost").await;
        // (No assertion; just verify it returns cleanly.)
    }

    #[test]
    fn test_deletion_summary_default_is_all_false() {
        let s = DeletionSummary::default();
        assert!(!s.discord_stopped);
        assert!(!s.telegram_stopped);
        assert!(!s.browsers_kicked);
        assert!(!s.in_memory_evicted);
        assert!(!s.files_wiped);
        assert!(!s.db_deleted);
    }
}
