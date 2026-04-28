//! In-memory cache of parsed per-user skill frontmatter.
//! Disk is the source of truth: each user's skills live at
//! `{workspace_root}/{user_id}/skills/{name}/SKILL.md`.
//!
//! The cache is invalidated by any write under `skills/` via
//! `is_under_skills_dir` — see A-8's write_file, A-9's edit_file,
//! A-10's delete_file, A-14's file_transfer.

use dashmap::DashMap;
use std::path::Path;
use std::sync::Arc;

use crate::context::SkillInfo;

#[derive(Default)]
pub struct SkillsCache {
    entries: DashMap<String, Arc<Vec<SkillInfo>>>,
}

impl SkillsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Invalidate the cached skill bundle for a user.
    /// Called from write_file / edit_file / delete_file / file_transfer
    /// when the resolved path is under `{workspace}/{user_id}/skills/`.
    pub fn invalidate(&self, user_id: &str) {
        self.entries.remove(user_id);
    }

    /// Return the cached skill list, loading from disk if not present.
    /// The returned Arc<Vec<SkillInfo>> is safe to clone cheaply.
    pub async fn get_or_load(&self, user_id: &str, workspace_root: &Path) -> Arc<Vec<SkillInfo>> {
        if let Some(existing) = self.entries.get(user_id) {
            return existing.clone();
        }
        let skills = load_skills_from_disk(workspace_root, user_id).await;
        let bundle = Arc::new(skills);
        self.entries.insert(user_id.to_string(), bundle.clone());
        bundle
    }
}

async fn load_skills_from_disk(workspace_root: &Path, user_id: &str) -> Vec<SkillInfo> {
    let skills_dir = workspace_root.join(user_id).join("skills");

    let mut entries = match tokio::fs::read_dir(&skills_dir).await {
        Ok(e) => e,
        // User hasn't been registered, or they deleted their skills dir.
        // Either way, empty list is correct.
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let file_type = match entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !file_type.is_dir() {
            continue;
        }

        let skill_dir_name = entry.file_name().to_string_lossy().to_string();
        let skill_md = entry.path().join("SKILL.md");
        let content = match tokio::fs::read_to_string(&skill_md).await {
            Ok(c) => c,
            Err(_) => {
                tracing::warn!(user_id, skill = %skill_dir_name, "skill directory missing SKILL.md, skipping");
                continue;
            }
        };

        let (name, description, always_on) = match parse_frontmatter(&content) {
            Ok(fm) => fm,
            Err(e) => {
                tracing::warn!(user_id, skill = %skill_dir_name, error = %e, "invalid SKILL.md frontmatter, skipping");
                continue;
            }
        };

        // Prefer frontmatter name; fall back to directory name if missing.
        let final_name = if name.is_empty() {
            skill_dir_name
        } else {
            name
        };

        // Optimization: on-demand skills don't need their body cached. context.rs
        // only reads SkillInfo.content for always-on skills; on-demand shows up in
        // the index as name + description only, and the agent loads the body via
        // read_file when needed.
        let cached_content = if always_on { content } else { String::new() };

        out.push(SkillInfo {
            name: final_name,
            description,
            always_on,
            content: cached_content,
        });
    }

    // Stable ordering: always-on first (so they appear first in the system prompt),
    // then alphabetical within each group.
    out.sort_by(|a, b| {
        b.always_on
            .cmp(&a.always_on)
            .then_with(|| a.name.cmp(&b.name))
    });

    out
}

/// Parse YAML frontmatter from a SKILL.md.
/// Expected format:
///     ---
///     name: my_skill
///     description: does X
///     always_on: false
///     ---
///     <body>
///
/// Returns (name, description, always_on). `name` may be empty (caller falls
/// back to the directory name). `description` defaults to empty string.
fn parse_frontmatter(content: &str) -> Result<(String, String, bool), &'static str> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix("---").ok_or("missing frontmatter")?;
    // The closing "---" can be at start of line after a newline.
    let end = rest.find("\n---").ok_or("missing closing ---")?;
    let fm = &rest[..end];

    let mut name = String::new();
    let mut description = String::new();
    let mut always_on = false;

    for line in fm.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("description:") {
            description = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("always_on:") {
            always_on = v.trim() == "true";
        }
    }

    Ok((name, description, always_on))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn write_skill(root: &Path, user_id: &str, name: &str, frontmatter: &str, body: &str) {
        let dir = root.join(user_id).join("skills").join(name);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let contents = format!("---\n{frontmatter}---\n{body}");
        tokio::fs::write(dir.join("SKILL.md"), contents)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_load_skills_from_disk_partitions_always_on() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "alice",
            "always1",
            "name: always1\ndescription: always loaded\nalways_on: true\n",
            "always body",
        )
        .await;
        write_skill(
            tmp.path(),
            "alice",
            "ondemand1",
            "name: ondemand1\ndescription: loaded on demand\nalways_on: false\n",
            "ondemand body",
        )
        .await;

        let skills = load_skills_from_disk(tmp.path(), "alice").await;
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "always1");
        assert!(skills[0].always_on);
        assert_eq!(skills[0].description, "always loaded");
        assert!(skills[0].content.contains("always body"));
        assert_eq!(skills[1].name, "ondemand1");
        assert!(!skills[1].always_on);
        assert_eq!(
            skills[1].content, "",
            "on-demand content should be empty in cache"
        );
    }

    #[tokio::test]
    async fn test_load_skills_missing_dir_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let skills = load_skills_from_disk(tmp.path(), "nobody").await;
        assert_eq!(skills.len(), 0);
    }

    #[tokio::test]
    async fn test_load_skills_skips_invalid_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("alice/skills/bad");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("SKILL.md"), "no frontmatter here\nraw content\n")
            .await
            .unwrap();

        write_skill(
            tmp.path(),
            "alice",
            "good",
            "name: good\ndescription: ok\nalways_on: false\n",
            "body",
        )
        .await;

        let skills = load_skills_from_disk(tmp.path(), "alice").await;
        // Only "good" loads; "bad" skipped with a warn log.
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "good");
    }

    #[tokio::test]
    async fn test_load_skills_falls_back_to_dir_name_when_frontmatter_name_missing() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "alice",
            "my_dir_name",
            "description: no name field\nalways_on: false\n",
            "body",
        )
        .await;

        let skills = load_skills_from_disk(tmp.path(), "alice").await;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my_dir_name");
    }

    #[tokio::test]
    async fn test_cache_returns_cached_on_second_call() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "alice",
            "a",
            "name: a\ndescription: first\nalways_on: false\n",
            "body",
        )
        .await;

        let cache = SkillsCache::new();
        let first = cache.get_or_load("alice", tmp.path()).await;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].description, "first");

        // Modify the file on disk. Without invalidate, the cache should return
        // the previously loaded result (not the new one).
        let dir = tmp.path().join("alice/skills/a");
        tokio::fs::write(
            dir.join("SKILL.md"),
            "---\nname: a\ndescription: updated\nalways_on: false\n---\nbody",
        )
        .await
        .unwrap();

        let second = cache.get_or_load("alice", tmp.path()).await;
        assert_eq!(
            second[0].description, "first",
            "cache should serve stale data until invalidated"
        );
    }

    #[tokio::test]
    async fn test_cache_reloads_after_invalidate() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "alice",
            "a",
            "name: a\ndescription: first\nalways_on: false\n",
            "body",
        )
        .await;

        let cache = SkillsCache::new();
        let _first = cache.get_or_load("alice", tmp.path()).await;

        // Modify the file and invalidate the cache.
        let dir = tmp.path().join("alice/skills/a");
        tokio::fs::write(
            dir.join("SKILL.md"),
            "---\nname: a\ndescription: updated\nalways_on: false\n---\nbody",
        )
        .await
        .unwrap();
        cache.invalidate("alice");

        let second = cache.get_or_load("alice", tmp.path()).await;
        assert_eq!(
            second[0].description, "updated",
            "cache should reload fresh data after invalidate"
        );
    }

    #[tokio::test]
    async fn test_cache_is_per_user() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "alice",
            "a",
            "name: a\ndescription: alice\nalways_on: false\n",
            "body",
        )
        .await;
        write_skill(
            tmp.path(),
            "bob",
            "b",
            "name: b\ndescription: bob\nalways_on: false\n",
            "body",
        )
        .await;

        let cache = SkillsCache::new();
        let alice = cache.get_or_load("alice", tmp.path()).await;
        let bob = cache.get_or_load("bob", tmp.path()).await;

        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].description, "alice");
        assert_eq!(bob.len(), 1);
        assert_eq!(bob[0].description, "bob");

        // Invalidating one user does not affect the other.
        cache.invalidate("alice");
        assert!(cache.entries.get("alice").is_none());
        assert!(cache.entries.get("bob").is_some());
    }

    #[tokio::test]
    async fn test_load_skills_sort_order_with_mixed_groups() {
        let tmp = TempDir::new().unwrap();
        // Create in random order; assert output has always-on first then alpha within each group.
        for (name, ao) in [
            ("zz_always", true),
            ("aa_always", true),
            ("mm_ondemand", false),
            ("bb_ondemand", false),
        ] {
            write_skill(
                tmp.path(),
                "alice",
                name,
                &format!("name: {name}\ndescription: d\nalways_on: {ao}\n"),
                "body",
            )
            .await;
        }

        let skills = load_skills_from_disk(tmp.path(), "alice").await;
        assert_eq!(skills.len(), 4);
        // Expected order: aa_always, zz_always, bb_ondemand, mm_ondemand.
        assert_eq!(skills[0].name, "aa_always");
        assert!(skills[0].always_on);
        assert_eq!(skills[1].name, "zz_always");
        assert!(skills[1].always_on);
        assert_eq!(skills[2].name, "bb_ondemand");
        assert!(!skills[2].always_on);
        assert_eq!(skills[3].name, "mm_ondemand");
        assert!(!skills[3].always_on);
    }
}
