use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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
    #[error("upload would exceed hard ceiling ({0} + {1} > {2})")]
    HardCeiling(u64, u64, u64),
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
        if bytes > self.per_upload_cap() {
            return Err(QuotaError::UploadTooLarge(bytes, self.per_upload_cap()));
        }
        let counter = self.usage_for(user_id);
        let current = counter.load(Ordering::SeqCst);
        if current > self.quota_bytes {
            return Err(QuotaError::SoftLocked(current, self.quota_bytes));
        }
        let new_usage = current + bytes;
        // Allow the upload even if it pushes over 100% (grace window);
        // soft-lock activates on the *next* write attempt.
        counter.store(new_usage, Ordering::SeqCst);
        Ok(())
    }

    pub fn record_delete(&self, user_id: &str, bytes_freed: u64) {
        let counter = self.usage_for(user_id);
        counter.fetch_sub(
            bytes_freed.min(counter.load(Ordering::SeqCst)),
            Ordering::SeqCst,
        );
    }

    pub fn current_usage(&self, user_id: &str) -> u64 {
        self.usage_for(user_id).load(Ordering::SeqCst)
    }

    pub fn quota_bytes(&self) -> u64 {
        self.quota_bytes
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
}
