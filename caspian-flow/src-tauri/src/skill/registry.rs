//! Skill registry — in-memory index of loaded skills with secondary indexes.
//!
//! Uses a single `RwLock<RegistryInner>` to guarantee atomic reads and writes
//! without deadlock risk. Internal helper methods take `&RegistryInner` directly,
//! never re-acquiring the lock.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use super::schema::Skill;

/// Inner state protected by the RwLock.
struct RegistryInner {
    /// Primary index: skill name → Skill.
    skills: HashMap<String, Skill>,
    /// Secondary index: category → skill names (for < 1ms category queries).
    by_category: HashMap<String, Vec<String>>,
    /// Secondary index: tag → skill names (for < 1ms tag queries).
    by_tag: HashMap<String, Vec<String>>,
}

impl RegistryInner {
    fn new() -> Self {
        Self {
            skills: HashMap::new(),
            by_category: HashMap::new(),
            by_tag: HashMap::new(),
        }
    }

    /// Register a skill, handling duplicate names by override.
    ///
    /// If a skill with the same name already exists:
    /// - A warning is logged with both old and new paths.
    /// - The old entry is removed from secondary indexes.
    /// - The new entry replaces the old one.
    /// - Physical files are not touched — only the in-memory index changes.
    fn register(&mut self, skill: Skill) {
        let name = skill.name.clone();

        // Check for duplicate and extract old skill data (clone to release borrow)
        if let Some(existing) = self.skills.get(&name) {
            let old_path = existing.path.clone();
            let old_category = existing.category.clone();
            let old_tags = existing.tags.clone();

            tracing::warn!(
                skill = %name,
                old_path = %old_path.display(),
                new_path = %skill.path.display(),
                "duplicate skill name — overriding with newer entry (in-memory only)"
            );

            // Remove old skill from secondary indexes
            self.remove_from_category_index(&name, &old_category);
            for tag in &old_tags {
                self.remove_from_tag_index(&name, tag);
            }
        }

        // Add to category index
        self.by_category
            .entry(skill.category.clone())
            .or_default()
            .push(name.clone());

        // Add to tag index
        for tag in &skill.tags {
            self.by_tag
                .entry(tag.clone())
                .or_default()
                .push(name.clone());
        }

        // Insert into primary index
        self.skills.insert(name, skill);
    }

    /// Remove a skill by name from the registry and all indexes.
    fn unregister(&mut self, name: &str) -> Option<Skill> {
        let skill = self.skills.remove(name)?;
        if !skill.category.is_empty() {
            self.remove_from_category_index(name, &skill.category);
        }
        for tag in &skill.tags {
            self.remove_from_tag_index(name, tag);
        }
        Some(skill)
    }

    /// Remove a skill name from the category index.
    fn remove_from_category_index(&mut self, name: &str, category: &str) {
        if let Some(names) = self.by_category.get_mut(category) {
            names.retain(|n| n != name);
            if names.is_empty() {
                self.by_category.remove(category);
            }
        }
    }

    /// Remove a skill name from the tag index.
    fn remove_from_tag_index(&mut self, name: &str, tag: &str) {
        if let Some(names) = self.by_tag.get_mut(tag) {
            names.retain(|n| n != name);
            if names.is_empty() {
                self.by_tag.remove(tag);
            }
        }
    }

    /// Rebuild secondary indexes from the primary skills map.
    #[allow(dead_code)]
    fn rebuild_indexes(&mut self) {
        self.by_category.clear();
        self.by_tag.clear();

        for (name, skill) in &self.skills {
            self.by_category
                .entry(skill.category.clone())
                .or_default()
                .push(name.clone());

            for tag in &skill.tags {
                self.by_tag
                    .entry(tag.clone())
                    .or_default()
                    .push(name.clone());
            }
        }
    }

    /// Get a skill by name (clone).
    fn get(&self, name: &str) -> Option<Skill> {
        self.skills.get(name).cloned()
    }

    /// List all skills.
    fn list_all(&self) -> Vec<Skill> {
        self.skills.values().cloned().collect()
    }

    /// List only enabled skills.
    fn list_enabled(&self) -> Vec<Skill> {
        self.skills
            .values()
            .filter(|s| s.enabled)
            .cloned()
            .collect()
    }

    /// List skill names by category.
    fn list_by_category(&self, category: &str) -> Vec<Skill> {
        self.by_category
            .get(category)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|n| self.skills.get(n))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List skill names by tag.
    fn list_by_tag(&self, tag: &str) -> Vec<Skill> {
        self.by_tag
            .get(tag)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|n| self.skills.get(n))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all categories.
    fn categories(&self) -> Vec<String> {
        self.by_category.keys().cloned().collect()
    }

    /// List all tags.
    fn tags(&self) -> Vec<String> {
        self.by_tag.keys().cloned().collect()
    }

    /// Search skills by text query across name, display_name, description, and tags.
    fn search(&self, query: &str) -> Vec<Skill> {
        let query_lower = query.to_lowercase();
        self.skills
            .values()
            .filter(|s| {
                s.name.to_lowercase().contains(&query_lower)
                    || s.display_name.to_lowercase().contains(&query_lower)
                    || s.description.to_lowercase().contains(&query_lower)
                    || s.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .cloned()
            .collect()
    }

    /// Count total skills.
    fn count(&self) -> usize {
        self.skills.len()
    }

    /// Count enabled skills.
    fn count_enabled(&self) -> usize {
        self.skills.values().filter(|s| s.enabled).count()
    }
}

/// Thread-safe skill registry with secondary indexes.
///
/// All public methods acquire the appropriate lock. Internal operations
/// are performed on `&RegistryInner` / `&mut RegistryInner` to avoid
/// re-entrant locking (no deadlock risk).
pub struct SkillRegistry {
    inner: RwLock<RegistryInner>,
}

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(RegistryInner::new()),
        }
    }

    /// Register a single skill.
    ///
    /// If a skill with the same name already exists, the new skill overrides
    /// the old one. A warning is logged with both the old and new paths.
    /// Physical files on disk are never touched.
    pub fn register(&self, skill: Skill) {
        let mut inner = self.inner.write();
        inner.register(skill);
    }

    /// Register multiple skills at once (replaces all existing entries).
    ///
    /// Duplicate names within the batch follow the "last wins" strategy:
    /// each duplicate logs a warning with old and new paths.
    pub fn register_all(&self, skills: Vec<Skill>) {
        let mut inner = self.inner.write();
        for skill in skills {
            inner.register(skill);
        }
    }

    /// Replace all skills in the registry with the given list.
    ///
    /// This clears the registry first, then registers all new skills.
    /// Duplicates within the batch follow the "last wins" strategy.
    pub fn replace_all(&self, skills: Vec<Skill>) {
        let mut inner = self.inner.write();
        inner.skills.clear();
        inner.by_category.clear();
        inner.by_tag.clear();
        for skill in skills {
            inner.register(skill);
        }
    }

    /// Unregister a skill by name.
    pub fn unregister(&self, name: &str) -> Option<Skill> {
        let mut inner = self.inner.write();
        inner.unregister(name)
    }

    /// Get a skill by name (clone).
    ///
    /// Complexity: O(1) — direct HashMap lookup.
    pub fn get(&self, name: &str) -> Option<Skill> {
        let inner = self.inner.read();
        inner.get(name)
    }

    /// Check if a skill exists by name.
    pub fn exists(&self, name: &str) -> bool {
        let inner = self.inner.read();
        inner.skills.contains_key(name)
    }

    /// List all skills.
    pub fn list_all(&self) -> Vec<Skill> {
        let inner = self.inner.read();
        inner.list_all()
    }

    /// List only enabled skills.
    pub fn list_enabled(&self) -> Vec<Skill> {
        let inner = self.inner.read();
        inner.list_enabled()
    }

    /// List skills by category.
    ///
    /// Complexity: O(k) where k is the number of skills in that category
    /// (uses a secondary index, no full scan).
    pub fn list_by_category(&self, category: &str) -> Vec<Skill> {
        let inner = self.inner.read();
        inner.list_by_category(category)
    }

    /// List skills by tag.
    ///
    /// Complexity: O(k) where k is the number of skills with that tag
    /// (uses a secondary index, no full scan).
    pub fn list_by_tag(&self, tag: &str) -> Vec<Skill> {
        let inner = self.inner.read();
        inner.list_by_tag(tag)
    }

    /// List all categories that have at least one skill.
    pub fn categories(&self) -> Vec<String> {
        let inner = self.inner.read();
        inner.categories()
    }

    /// List all tags that have at least one skill.
    pub fn tags(&self) -> Vec<String> {
        let inner = self.inner.read();
        inner.tags()
    }

    /// Search skills by text query (name, display_name, description, tags).
    pub fn search(&self, query: &str) -> Vec<Skill> {
        let inner = self.inner.read();
        inner.search(query)
    }

    /// Enable a skill by name.
    ///
    /// Returns `true` if the skill was found and enabled, `false` if not found.
    pub fn enable(&self, name: &str) -> bool {
        let mut inner = self.inner.write();
        if let Some(skill) = inner.skills.get_mut(name) {
            skill.enabled = true;
            tracing::info!(skill = %name, "skill enabled");
            true
        } else {
            false
        }
    }

    /// Disable a skill by name.
    ///
    /// Returns `true` if the skill was found and disabled, `false` if not found.
    pub fn disable(&self, name: &str) -> bool {
        let mut inner = self.inner.write();
        if let Some(skill) = inner.skills.get_mut(name) {
            skill.enabled = false;
            tracing::info!(skill = %name, "skill disabled");
            true
        } else {
            false
        }
    }

    /// Set the enabled state of a skill.
    pub fn set_enabled(&self, name: &str, enabled: bool) -> bool {
        if enabled {
            self.enable(name)
        } else {
            self.disable(name)
        }
    }

    /// Total number of registered skills.
    pub fn count(&self) -> usize {
        let inner = self.inner.read();
        inner.count()
    }

    /// Number of enabled skills.
    pub fn count_enabled(&self) -> usize {
        let inner = self.inner.read();
        inner.count_enabled()
    }

    /// Clear all skills from the registry.
    pub fn clear(&self) {
        let mut inner = self.inner.write();
        inner.skills.clear();
        inner.by_category.clear();
        inner.by_tag.clear();
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A thread-safe shared registry.
pub type SharedSkillRegistry = Arc<SkillRegistry>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::{Skill, SkillRuntime, SkillRuntimeType};
    use std::path::PathBuf;

    fn make_skill(name: &str, category: &str, tags: Vec<&str>) -> Skill {
        Skill {
            mcp: None,
            schema_version: "1.0".to_string(),
            name: name.to_string(),
            display_name: name.replace('_', " "),
            version: "1.0.0".to_string(),
            description: format!("Test skill {name}"),
            category: category.to_string(),
            trigger_phrases: vec!["test".to_string()],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Python,
                entry: "script.py".to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            permissions: Default::default(),
            tags: tags.into_iter().map(String::from).collect(),
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from(format!("/skills/{name}")),
        }
    }

    #[test]
    fn test_empty_registry() {
        let registry = SkillRegistry::new();
        assert_eq!(registry.count(), 0);
        assert!(registry.list_all().is_empty());
        assert!(registry.list_enabled().is_empty());
    }

    #[test]
    fn test_register_and_get() {
        let registry = SkillRegistry::new();
        let skill = make_skill("read_file", "file-system", vec!["file", "read"]);
        registry.register(skill);

        assert_eq!(registry.count(), 1);
        let found = registry.get("read_file").unwrap();
        assert_eq!(found.name, "read_file");
        assert_eq!(found.category, "file-system");
    }

    #[test]
    fn test_get_nonexistent() {
        let registry = SkillRegistry::new();
        assert!(registry.get("nonexistent").is_none());
        assert!(!registry.exists("nonexistent"));
    }

    #[test]
    fn test_register_multiple() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "file-system", vec!["file"]));
        registry.register(make_skill("skill_b", "network", vec!["http"]));
        registry.register(make_skill("skill_c", "utility", vec!["file", "http"]));

        assert_eq!(registry.count(), 3);
        assert!(registry.exists("skill_a"));
        assert!(registry.exists("skill_b"));
        assert!(registry.exists("skill_c"));
    }

    #[test]
    fn test_duplicate_override() {
        let registry = SkillRegistry::new();

        let mut skill1 = make_skill("dup_skill", "file-system", vec!["file"]);
        skill1.path = PathBuf::from("/skills/dup_v1");

        let mut skill2 = make_skill("dup_skill", "network", vec!["http"]);
        skill2.path = PathBuf::from("/skills/dup_v2");

        registry.register(skill1);
        assert_eq!(registry.count(), 1);

        // Override — should replace, not add
        registry.register(skill2);
        assert_eq!(registry.count(), 1);

        // The new skill should be the one returned
        let found = registry.get("dup_skill").unwrap();
        assert_eq!(found.category, "network");
        assert_eq!(found.path, PathBuf::from("/skills/dup_v2"));

        // Old category index should be cleaned up
        assert!(registry.list_by_category("file-system").is_empty());
        assert_eq!(registry.list_by_category("network").len(), 1);

        // Old tag index should be cleaned up
        assert!(registry.list_by_tag("file").is_empty());
        assert_eq!(registry.list_by_tag("http").len(), 1);
    }

    #[test]
    fn test_list_by_category() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "file-system", vec![]));
        registry.register(make_skill("skill_b", "file-system", vec![]));
        registry.register(make_skill("skill_c", "network", vec![]));

        let fs_skills = registry.list_by_category("file-system");
        assert_eq!(fs_skills.len(), 2);

        let net_skills = registry.list_by_category("network");
        assert_eq!(net_skills.len(), 1);

        assert!(registry.list_by_category("nonexistent").is_empty());
    }

    #[test]
    fn test_list_by_tag() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "file-system", vec!["file", "read"]));
        registry.register(make_skill("skill_b", "network", vec!["file", "http"]));
        registry.register(make_skill("skill_c", "utility", vec!["http"]));

        let file_skills = registry.list_by_tag("file");
        assert_eq!(file_skills.len(), 2);

        let http_skills = registry.list_by_tag("http");
        assert_eq!(http_skills.len(), 2);

        let read_skills = registry.list_by_tag("read");
        assert_eq!(read_skills.len(), 1);

        assert!(registry.list_by_tag("nonexistent").is_empty());
    }

    #[test]
    fn test_categories_and_tags() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "file-system", vec!["file", "read"]));
        registry.register(make_skill("skill_b", "network", vec!["http"]));

        let cats = registry.categories();
        assert_eq!(cats.len(), 2);
        assert!(cats.contains(&"file-system".to_string()));
        assert!(cats.contains(&"network".to_string()));

        let tags = registry.tags();
        assert_eq!(tags.len(), 3);
        assert!(tags.contains(&"file".to_string()));
        assert!(tags.contains(&"read".to_string()));
        assert!(tags.contains(&"http".to_string()));
    }

    #[test]
    fn test_enable_disable() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "utility", vec![]));

        assert!(registry.list_enabled().len() == 1);

        // Disable
        assert!(registry.disable("skill_a"));
        assert_eq!(registry.count(), 1);
        assert_eq!(registry.count_enabled(), 0);
        assert!(registry.list_enabled().is_empty());

        // The skill should still be in list_all
        assert_eq!(registry.list_all().len(), 1);

        // Enable
        assert!(registry.enable("skill_a"));
        assert_eq!(registry.count_enabled(), 1);
        assert_eq!(registry.list_enabled().len(), 1);
    }

    #[test]
    fn test_enable_nonexistent() {
        let registry = SkillRegistry::new();
        assert!(!registry.enable("nonexistent"));
        assert!(!registry.disable("nonexistent"));
    }

    #[test]
    fn test_set_enabled() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "utility", vec![]));

        assert!(registry.set_enabled("skill_a", false));
        assert_eq!(registry.count_enabled(), 0);

        assert!(registry.set_enabled("skill_a", true));
        assert_eq!(registry.count_enabled(), 1);
    }

    #[test]
    fn test_search() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("read_file", "file-system", vec!["file"]));
        registry.register(make_skill("write_file", "file-system", vec!["file"]));
        registry.register(make_skill("send_email", "network", vec!["email"]));

        let results = registry.search("file");
        assert_eq!(results.len(), 2);

        let results = registry.search("read");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "read_file");

        let results = registry.search("email");
        assert_eq!(results.len(), 1);

        let results = registry.search("nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_unregister() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "file-system", vec!["file"]));
        registry.register(make_skill("skill_b", "network", vec!["http"]));

        let removed = registry.unregister("skill_a");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "skill_a");
        assert_eq!(registry.count(), 1);
        assert!(!registry.exists("skill_a"));

        // Category and tag indexes should be updated
        assert!(registry.list_by_category("file-system").is_empty());
        assert!(registry.list_by_tag("file").is_empty());
        assert_eq!(registry.list_by_category("network").len(), 1);
    }

    #[test]
    fn test_unregister_nonexistent() {
        let registry = SkillRegistry::new();
        assert!(registry.unregister("nonexistent").is_none());
    }

    #[test]
    fn test_replace_all() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("old_skill", "utility", vec![]));

        let new_skills = vec![
            make_skill("new_a", "file-system", vec!["file"]),
            make_skill("new_b", "network", vec!["http"]),
        ];

        registry.replace_all(new_skills);

        assert_eq!(registry.count(), 2);
        assert!(!registry.exists("old_skill"));
        assert!(registry.exists("new_a"));
        assert!(registry.exists("new_b"));
    }

    #[test]
    fn test_clear() {
        let registry = SkillRegistry::new();
        registry.register(make_skill("skill_a", "utility", vec![]));
        registry.register(make_skill("skill_b", "network", vec![]));

        registry.clear();
        assert_eq!(registry.count(), 0);
        assert!(registry.list_all().is_empty());
        assert!(registry.categories().is_empty());
        assert!(registry.tags().is_empty());
    }

    #[test]
    fn test_register_all_with_duplicates() {
        let registry = SkillRegistry::new();

        let skills = vec![
            make_skill("dup", "file-system", vec!["file"]),
            make_skill("other", "network", vec!["http"]),
            make_skill("dup", "utility", vec!["test"]),
        ];

        registry.register_all(skills);

        // "dup" should be overridden — only 2 unique names
        assert_eq!(registry.count(), 2);

        let found = registry.get("dup").unwrap();
        assert_eq!(found.category, "utility"); // last one wins
        assert_eq!(found.tags, vec!["test"]);
    }

    #[test]
    fn test_concurrent_reads() {
        let registry = Arc::new(SkillRegistry::new());
        registry.register(make_skill("skill_a", "utility", vec!["test"]));

        let registry_clone = registry.clone();
        let handle = std::thread::spawn(move || {
            let skill = registry_clone.get("skill_a");
            assert!(skill.is_some());
        });

        // Concurrent read from main thread
        let skill = registry.get("skill_a");
        assert!(skill.is_some());

        handle.join().unwrap();
    }

    #[test]
    fn test_concurrent_write_and_read() {
        let registry = Arc::new(SkillRegistry::new());

        let writer = {
            let registry = registry.clone();
            std::thread::spawn(move || {
                for i in 0..10 {
                    registry.register(make_skill(&format!("skill_{i}"), "test", vec![]));
                }
            })
        };

        // Reader can read while writer is writing
        writer.join().unwrap();

        assert_eq!(registry.count(), 10);
    }

    #[test]
    fn test_category_index_cleanup_on_override() {
        let registry = SkillRegistry::new();

        // Register with category A
        let mut skill1 = make_skill("test", "category_a", vec!["tag1"]);
        skill1.path = PathBuf::from("/v1");
        registry.register(skill1);

        assert_eq!(registry.list_by_category("category_a").len(), 1);

        // Override with category B
        let mut skill2 = make_skill("test", "category_b", vec!["tag2"]);
        skill2.path = PathBuf::from("/v2");
        registry.register(skill2);

        // Category A should be empty now
        assert!(registry.list_by_category("category_a").is_empty());
        assert_eq!(registry.list_by_category("category_b").len(), 1);

        // Tag1 should be gone, tag2 should be present
        assert!(registry.list_by_tag("tag1").is_empty());
        assert_eq!(registry.list_by_tag("tag2").len(), 1);
    }
}
