//! Router index — stores trigger-phrase embeddings for all enabled skills.
//!
//! The index is built by:
//! 1. Collecting all enabled skills from the `SkillRegistry`
//! 2. Batch-embedding their `trigger_phrases`
//! 3. Grouping embeddings by skill name
//!
//! The index is immutable once built. When skills change, call `rebuild()`
//! to create a new index and atomically replace the old one.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::knowledge::similarity::cosine_similarity;
use crate::router::provider::EmbeddingProvider;
use crate::skill::registry::SkillRegistry;
use crate::skill::schema::Skill;
use crate::types::AppResult;

/// An entry in the router index: a skill and its trigger-phrase embeddings.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub skill: Skill,
    pub trigger_embeddings: Vec<Vec<f32>>,
}

/// The router index — maps skill names to their trigger-phrase embeddings.
#[derive(Debug, Clone)]
pub struct RouterIndex {
    /// skill_name → index entry
    entries: HashMap<String, IndexEntry>,
    /// Embedding dimension (0 if index is empty).
    dimension: usize,
}

impl RouterIndex {
    /// Create an empty index.
    pub fn empty() -> Self {
        Self {
            entries: HashMap::new(),
            dimension: 0,
        }
    }

    /// Build a new index from the given skills using the provided embedding provider.
    ///
    /// Collects all trigger phrases from enabled skills, batch-embeds them,
    /// and groups the results by skill name.
    pub async fn build(
        registry: &SkillRegistry,
        provider: &dyn EmbeddingProvider,
    ) -> AppResult<Self> {
        let skills = registry.list_enabled();
        Self::build_from_skills(&skills, provider).await
    }

    /// Build a new index from a slice of skills.
    pub async fn build_from_skills(
        skills: &[Skill],
        provider: &dyn EmbeddingProvider,
    ) -> AppResult<Self> {
        // Collect all trigger phrases with their owning skill name
        let mut all_phrases: Vec<String> = Vec::new();
        let mut phrase_ranges: Vec<(String, usize, usize)> = Vec::new();

        for skill in skills {
            // Skip disabled skills
            if !skill.enabled {
                continue;
            }

            if skill.trigger_phrases.is_empty() {
                tracing::warn!(
                    skill = %skill.name,
                    "skill has no trigger phrases, skipping from index"
                );
                continue;
            }

            let start = all_phrases.len();
            for phrase in &skill.trigger_phrases {
                all_phrases.push(phrase.clone());
            }
            let end = all_phrases.len();
            phrase_ranges.push((skill.name.clone(), start, end));
        }

        if all_phrases.is_empty() {
            tracing::warn!("no trigger phrases found in any skill — index is empty");
            return Ok(Self::empty());
        }

        // Batch embed all phrases
        let embeddings = provider.embed_batch(&all_phrases).await?;

        let dimension = embeddings.first().map(|e| e.len()).unwrap_or(0);

        // Group embeddings by skill
        let mut entries: HashMap<String, IndexEntry> = HashMap::new();
        for (skill_name, start, end) in phrase_ranges {
            let skill = skills
                .iter()
                .find(|s| s.name == skill_name)
                .cloned()
                .expect("skill must exist");

            let trigger_embeddings = embeddings[start..end].to_vec();
            entries.insert(
                skill_name,
                IndexEntry {
                    skill,
                    trigger_embeddings,
                },
            );
        }

        tracing::info!(
            skills = entries.len(),
            dimension,
            "router index built successfully"
        );

        Ok(Self { entries, dimension })
    }

    /// Get an entry by skill name.
    pub fn get(&self, skill_name: &str) -> Option<&IndexEntry> {
        self.entries.get(skill_name)
    }

    /// Number of skills in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the embedding dimension.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Get all skill names in the index.
    pub fn skill_names(&self) -> Vec<&str> {
        self.entries.keys().map(|s| s.as_str()).collect()
    }

    /// Match a query embedding against all skills in the index.
    ///
    /// For each skill, computes the **maximum** cosine similarity across all
    /// of its trigger phrases (max-phrase strategy).
    ///
    /// Returns a list of `(skill_name, max_similarity)` sorted by descending similarity.
    pub fn match_query(&self, query_embedding: &[f32]) -> Vec<MatchResult> {
        let mut results: Vec<MatchResult> = self
            .entries
            .values()
            .map(|entry| {
                let max_sim = entry
                    .trigger_embeddings
                    .iter()
                    .map(|emb| cosine_similarity(query_embedding, emb))
                    .fold(f32::MIN, f32::max);

                let best_phrase_idx = entry
                    .trigger_embeddings
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| {
                        let sim_a = cosine_similarity(query_embedding, a);
                        let sim_b = cosine_similarity(query_embedding, b);
                        sim_a
                            .partial_cmp(&sim_b)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);

                MatchResult {
                    skill_name: entry.skill.name.clone(),
                    raw_score: if max_sim == f32::MIN { 0.0 } else { max_sim },
                    best_phrase_idx,
                }
            })
            .collect();

        // Sort by descending raw score
        results.sort_by(|a, b| {
            b.raw_score
                .partial_cmp(&a.raw_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }
}

/// A single match result from the index.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// The skill name.
    pub skill_name: String,
    /// The maximum cosine similarity across all trigger phrases.
    pub raw_score: f32,
    /// Index of the best-matching trigger phrase.
    pub best_phrase_idx: usize,
}

/// Thread-safe wrapper around `RouterIndex` for concurrent access.
///
/// Uses `arc-swap` for lock-free reads with atomic index replacement.
pub struct SharedRouterIndex {
    inner: Arc<RwLock<RouterIndex>>,
}

impl SharedRouterIndex {
    /// Create a new shared index, initially empty.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RouterIndex::empty())),
        }
    }

    /// Rebuild the index from the registry.
    pub async fn rebuild(
        &self,
        registry: &SkillRegistry,
        provider: &dyn EmbeddingProvider,
    ) -> AppResult<()> {
        let new_index = RouterIndex::build(registry, provider).await?;
        let count = new_index.len();
        *self.inner.write() = new_index;
        tracing::info!(skills = count, "router index rebuilt");
        Ok(())
    }

    /// Rebuild the index from a slice of skills.
    pub async fn rebuild_from_skills(
        &self,
        skills: &[Skill],
        provider: &dyn EmbeddingProvider,
    ) -> AppResult<()> {
        let new_index = RouterIndex::build_from_skills(skills, provider).await?;
        let count = new_index.len();
        *self.inner.write() = new_index;
        tracing::info!(skills = count, "router index rebuilt from skills slice");
        Ok(())
    }

    /// Get a clone of the current index for read-only access.
    pub fn snapshot(&self) -> RouterIndex {
        self.inner.read().clone()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }

    /// Get the number of skills in the index.
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Match a query embedding against the current index.
    pub fn match_query(&self, query_embedding: &[f32]) -> Vec<MatchResult> {
        self.inner.read().match_query(query_embedding)
    }
}

impl Default for SharedRouterIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SharedRouterIndex {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::provider::MockEmbeddingProvider;
    use crate::skill::schema::{Skill, SkillRuntime, SkillRuntimeType};
    use std::path::PathBuf;

    fn make_skill(name: &str, triggers: &[&str]) -> Skill {
        Skill {
            mcp: None,
            schema_version: "1.0".to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Test skill {name}"),
            category: "test".to_string(),
            trigger_phrases: triggers.iter().map(|s| s.to_string()).collect(),
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Python,
                entry: "script.py".to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            permissions: Default::default(),
            tags: vec![],
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from(format!("/skills/{name}")),
        }
    }

    #[tokio::test]
    async fn test_build_empty() {
        let provider = MockEmbeddingProvider::new(64);
        let skills: Vec<Skill> = vec![];
        let index = RouterIndex::build_from_skills(&skills, &provider)
            .await
            .unwrap();
        assert!(index.is_empty());
        assert_eq!(index.dimension(), 0);
    }

    #[tokio::test]
    async fn test_build_with_skills() {
        let provider = MockEmbeddingProvider::new(64);
        let skills = vec![
            make_skill("read_file", &["读取文件", "打开文件", "read file"]),
            make_skill("write_file", &["写入文件", "保存文件", "write file"]),
        ];
        let index = RouterIndex::build_from_skills(&skills, &provider)
            .await
            .unwrap();

        assert_eq!(index.len(), 2);
        assert_eq!(index.dimension(), 64);
        assert!(index.get("read_file").is_some());
        assert!(index.get("write_file").is_some());
    }

    #[tokio::test]
    async fn test_build_skips_no_triggers() {
        let provider = MockEmbeddingProvider::new(64);
        let skills = vec![make_skill("with_triggers", &["test phrase"]), {
            let mut s = make_skill("no_triggers", &[]);
            s.trigger_phrases = vec![];
            s
        }];
        let index = RouterIndex::build_from_skills(&skills, &provider)
            .await
            .unwrap();

        assert_eq!(index.len(), 1);
        assert!(index.get("with_triggers").is_some());
        assert!(index.get("no_triggers").is_none());
    }

    #[tokio::test]
    async fn test_match_query() {
        let provider = MockEmbeddingProvider::new(64);
        let skills = vec![
            make_skill("read_file", &["读取文件", "read file"]),
            make_skill("write_file", &["写入文件", "write file"]),
            make_skill("send_email", &["发送邮件", "send email"]),
        ];
        let index = RouterIndex::build_from_skills(&skills, &provider)
            .await
            .unwrap();

        let query = provider.pseudo_embed_public("读取文件");
        let results = index.match_query(&query);

        assert_eq!(results.len(), 3);
        // The read_file skill should be top-1 since the query matches its trigger
        assert_eq!(results[0].skill_name, "read_file");
        assert!(results[0].raw_score > 0.5);
    }

    #[tokio::test]
    async fn test_match_query_empty_index() {
        let index = RouterIndex::empty();
        let query = vec![1.0, 0.0, 0.0];
        let results = index.match_query(&query);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_shared_index_rebuild() {
        let shared = SharedRouterIndex::new();
        assert!(shared.is_empty());

        let provider = MockEmbeddingProvider::new(64);
        let skills = vec![make_skill("test_skill", &["test"])];

        shared
            .rebuild_from_skills(&skills, &provider)
            .await
            .unwrap();

        assert_eq!(shared.len(), 1);
        assert!(!shared.is_empty());
    }

    #[tokio::test]
    async fn test_shared_index_match() {
        let shared = SharedRouterIndex::new();
        let provider = MockEmbeddingProvider::new(64);
        let skills = vec![
            make_skill("read_file", &["读取文件"]),
            make_skill("write_file", &["写入文件"]),
        ];

        shared
            .rebuild_from_skills(&skills, &provider)
            .await
            .unwrap();

        let query = provider.pseudo_embed_public("读取文件");
        let results = shared.match_query(&query);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].skill_name, "read_file");
    }
}
