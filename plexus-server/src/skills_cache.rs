//! In-memory cache of parsed per-user skill frontmatter.
//! Placeholder implementation — fleshed out in Task A-16 with
//! disk-as-truth parsing and cache invalidation.

use dashmap::DashMap;

#[derive(Default)]
pub struct SkillsCache {
    // user_id -> placeholder. A-16 replaces `()` with `Arc<SkillsBundle>`.
    _inner: DashMap<String, ()>,
}

impl SkillsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Invalidate the cached skill bundle for a user. No-op until A-16 lands.
    pub fn invalidate(&self, _user_id: &str) {
        // Placeholder — no cache to clear yet.
    }
}
