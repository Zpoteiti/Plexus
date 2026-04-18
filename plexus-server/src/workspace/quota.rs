use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct QuotaCache {
    /// user_id -> current usage in bytes
    usage: DashMap<String, Arc<AtomicU64>>,
    /// Total quota per user in bytes.
    quota_bytes: u64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum QuotaError {
    #[error("upload exceeds per-upload cap ({0} bytes; cap {1} bytes)")]
    UploadTooLarge(u64, u64),
    #[error("workspace is soft-locked (usage {0} > quota {1}); delete files to continue")]
    SoftLocked(u64, u64),
    // HardCeiling removed — dead code. See 2026-04-17 code review.
    // Can be re-added in ~4 lines if a consumer appears.
}

impl QuotaCache {
    pub fn new(quota_bytes: u64) -> Self {
        Self {
            usage: DashMap::new(),
            quota_bytes,
        }
    }

    pub fn per_upload_cap(&self) -> u64 {
        self.quota_bytes * 4 / 5 // 80%
    }

    fn usage_for(&self, user_id: &str) -> Arc<AtomicU64> {
        self.usage
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone()
    }

    /// Check an incoming upload. Returns Ok if allowed; reserves the bytes by
    /// incrementing the usage counter atomically.
    pub fn check_and_reserve_upload(&self, user_id: &str, bytes: u64) -> Result<(), QuotaError> {
        let cap = self.per_upload_cap();
        if bytes > cap {
            return Err(QuotaError::UploadTooLarge(bytes, cap));
        }
        let counter = self.usage_for(user_id);
        let quota = self.quota_bytes;
        let update_result = counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            if current > quota {
                None // soft-locked — refuse
            } else {
                Some(current.saturating_add(bytes)) // reserve (grace window allows exceed)
            }
        });
        match update_result {
            Ok(_) => Ok(()),
            Err(current) => Err(QuotaError::SoftLocked(current, quota)),
        }
    }

    pub fn record_delete(&self, user_id: &str, bytes_freed: u64) {
        let counter = self.usage_for(user_id);
        let _ = counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            Some(current.saturating_sub(bytes_freed))
        });
    }

    pub fn current_usage(&self, user_id: &str) -> u64 {
        self.usage_for(user_id).load(Ordering::SeqCst)
    }

    pub fn quota_bytes(&self) -> u64 {
        self.quota_bytes
    }

    /// Remove all quota tracking for a user. Called when the user's account is
    /// deleted. After this, subsequent `check_and_reserve_upload` for the same
    /// user_id starts fresh from zero (though the account itself should be
    /// gone by that point).
    pub fn forget_user(&self, user_id: &str) {
        self.usage.remove(user_id);
    }

    /// Walks the workspace root and primes the usage cache for every existing user dir.
    /// Call once at server startup.
    pub async fn initialize_from_disk(
        &self,
        workspace_root: &std::path::Path,
    ) -> std::io::Result<()> {
        let mut entries = match tokio::fs::read_dir(workspace_root).await {
            Ok(e) => e,
            // If the workspace root doesn't exist yet (fresh install), that's fine.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let user_id = entry.file_name().to_string_lossy().to_string();
            let bytes = walk_dir_bytes(&entry.path()).await?;
            self.usage_for(&user_id).store(bytes, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// Sum the sizes of every regular file under `path`. Used by quota init and delete_file.
pub(crate) async fn walk_dir_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total += entry.metadata().await?.len();
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
impl QuotaCache {
    /// Directly increment the usage counter for a user. Used in unit tests
    /// to simulate a prior upload without going through the real upload flow.
    pub fn reserve_for_test(&self, user_id: &str, bytes: u64) {
        let entry = self
            .usage
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(bytes, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_exceeding_per_upload_cap_rejected() {
        let q = QuotaCache::new(5_000_000_000); // 5 GB
        let result = q.check_and_reserve_upload("alice", 4_500_000_000);
        assert!(matches!(result, Err(QuotaError::UploadTooLarge(_, _))));
    }

    #[test]
    fn test_upload_at_per_upload_cap_allowed() {
        let q = QuotaCache::new(5_000_000_000);
        let result = q.check_and_reserve_upload("alice", 4_000_000_000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_grace_window_allows_exceeding_100_percent() {
        let q = QuotaCache::new(5_000_000_000);
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap(); // 80%
        // Next upload would push to 160% — but allow (grace).
        let result = q.check_and_reserve_upload("alice", 4_000_000_000);
        assert!(result.is_ok());
        assert_eq!(q.current_usage("alice"), 8_000_000_000);
    }

    #[test]
    fn test_soft_lock_after_over_quota() {
        let q = QuotaCache::new(5_000_000_000);
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap(); // now over
        let result = q.check_and_reserve_upload("alice", 100);
        assert!(matches!(result, Err(QuotaError::SoftLocked(_, _))));
    }

    #[test]
    fn test_delete_releases_soft_lock() {
        let q = QuotaCache::new(5_000_000_000);
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();
        assert!(matches!(
            q.check_and_reserve_upload("alice", 100),
            Err(QuotaError::SoftLocked(_, _))
        ));
        q.record_delete("alice", 4_000_000_000); // drop to 4 GB
        let result = q.check_and_reserve_upload("alice", 100);
        assert!(result.is_ok());
    }

    #[test]
    fn test_forget_user_clears_counter() {
        let q = QuotaCache::new(5_000_000_000);
        q.check_and_reserve_upload("alice", 1_000).unwrap();
        assert_eq!(q.current_usage("alice"), 1_000);
        q.forget_user("alice");
        // Subsequent calls start from zero.
        assert_eq!(q.current_usage("alice"), 0);
    }

    #[test]
    fn test_concurrent_reservations_no_lost_update() {
        use std::sync::Arc as StdArc;
        use std::thread;

        let q = StdArc::new(QuotaCache::new(10_000)); // 10 KB quota
        // per-upload cap = 8000
        let mut handles = Vec::new();
        for _ in 0..100 {
            let q = q.clone();
            handles.push(thread::spawn(move || {
                // Each reserves 50 bytes. 100 threads × 50 = 5000 bytes total.
                let _ = q.check_and_reserve_upload("alice", 50);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // With the old load+store, concurrent writers lose updates and the
        // counter sits well below 5000. With fetch_update, every increment
        // is preserved.
        assert_eq!(q.current_usage("alice"), 5000);
    }
}
