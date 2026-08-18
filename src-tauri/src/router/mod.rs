//! Semantic Router module — the core skill-matching engine.
//!
//! The `SemanticRouter` is the main entry point for routing user queries
//! to skills. It integrates:
//! - An [`EmbeddingProvider`] for query vectorization
//! - A [`SharedRouterIndex`] for skill trigger-phrase embeddings
//! - [`SkillPreferences`] for weight-based tuning
//! - [`FeedbackStore`] for user feedback recording
//! - [`ColdStartTracker`] for cold-start correction collection
//!
//! ## Usage
//!
//! ```no_run
//! use caspian_flow::router::SemanticRouter;
//! use caspian_flow::router::provider::MockEmbeddingProvider;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let provider = std::sync::Arc::new(MockEmbeddingProvider::new(128));
//! let router = SemanticRouter::new(provider);
//!
//! // Route a query
//! let result = router.route("读取文件").await?;
//! # Ok(())
//! # }
//! ```

pub mod feedback;
pub mod health;
pub mod index;
pub mod model_router;
pub mod preferences;
pub mod prompt_templates;
pub mod provider;
pub mod providers;
pub mod semantic;
pub mod slot_filler;
pub mod types;

pub use feedback::{ColdStartTracker, FeedbackStore};
pub use health::{HealthChecker, HttpHealthChecker, InMemoryHealthChecker};
pub use index::{IndexEntry, MatchResult, RouterIndex, SharedRouterIndex};
pub use model_router::{ModelRouter, ModelRouterError, SelectionStrategy};
pub use preferences::SkillPreferences;
pub use prompt_templates::{build_correction_prompt, build_extraction_prompt};
pub use provider::{EmbeddingProvider, EmbeddingServiceAdapter, MockEmbeddingProvider};
pub use providers::{
    provider_from_config, resolve_preset, AnthropicProvider, AuthScheme, CustomProvider,
    GLMProvider, OpenAICompatibleProvider,
};
pub use semantic::{route, route_text, DEFAULT_TOP_K};
pub use slot_filler::{
    apply_defaults, assess_complexity, check_schema_keywords, extract_json, select_model_size,
    validate_against_schema, LlmProvider, MissingField, MockLlmProvider, ModelSize,
    SchemaComplexity, SchemaValidationError, SlotFillResult, SlotFiller, SlotFillerConfig,
    SUPPORTED_KEYWORDS,
};
pub use types::{
    CorrectionSample, FeedbackRecord, FeedbackType, RouteResult, ScoredSkill, Sensitivity,
};

use std::sync::Arc;

use parking_lot::RwLock;

use crate::skill::registry::SkillRegistry;
use crate::types::AppResult;

/// The central semantic router — integrates all routing components.
pub struct SemanticRouter {
    /// The embedding provider (real or mock).
    provider: Arc<dyn EmbeddingProvider>,
    /// Shared skill trigger-phrase index.
    index: SharedRouterIndex,
    /// Skill preferences (weights + sensitivity).
    preferences: RwLock<SkillPreferences>,
    /// Feedback store.
    feedback: FeedbackStore,
    /// Cold-start tracker.
    cold_start: ColdStartTracker,
    /// Maximum candidates to return in `Candidates` results.
    top_k: usize,
}

impl SemanticRouter {
    /// Create a new router with the given embedding provider.
    ///
    /// The router starts with an empty index — call `rebuild_index()` to
    /// populate it from a skill registry.
    pub fn new(provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            provider,
            index: SharedRouterIndex::new(),
            preferences: RwLock::new(SkillPreferences::new()),
            feedback: FeedbackStore::new(),
            cold_start: ColdStartTracker::new(),
            top_k: DEFAULT_TOP_K,
        }
    }

    /// Create a new router with custom top-K and sensitivity.
    pub fn with_config(
        provider: Arc<dyn EmbeddingProvider>,
        top_k: usize,
        sensitivity: Sensitivity,
    ) -> Self {
        let mut prefs = SkillPreferences::new();
        prefs.set_sensitivity(sensitivity);

        Self {
            provider,
            index: SharedRouterIndex::new(),
            preferences: RwLock::new(prefs),
            feedback: FeedbackStore::new(),
            cold_start: ColdStartTracker::new(),
            top_k,
        }
    }

    /// Rebuild the skill trigger-phrase index from a registry.
    pub async fn rebuild_index(&self, registry: &SkillRegistry) -> AppResult<()> {
        self.index.rebuild(registry, self.provider.as_ref()).await
    }

    /// Rebuild the index from a slice of skills (useful for testing).
    pub async fn rebuild_index_from_skills(
        &self,
        skills: &[crate::skill::schema::Skill],
    ) -> AppResult<()> {
        self.index
            .rebuild_from_skills(skills, self.provider.as_ref())
            .await
    }

    /// Route a user query to the best-matching skill(s).
    ///
    /// This is the main entry point for semantic routing.
    pub async fn route(&self, text: &str) -> AppResult<RouteResult> {
        // Increment cold-start usage counter
        self.cold_start.increment_usage();

        let prefs = self.preferences.read().clone();
        let result = route_text(
            text,
            self.provider.as_ref(),
            &self.index.snapshot(),
            &prefs,
            self.top_k,
        )
        .await?;

        // Log the routing decision
        match &result {
            RouteResult::DirectMatch { skill, score, .. } => {
                tracing::info!(
                    query = text,
                    matched_skill = %skill.name,
                    score,
                    "direct match"
                );
            }
            RouteResult::Candidates { skills, top_score } => {
                let names: Vec<&str> = skills.iter().map(|s| s.skill.name.as_str()).collect();
                tracing::info!(
                    query = text,
                    candidates = ?names,
                    top_score,
                    "candidate list"
                );
            }
            RouteResult::NoMatch { top_score, .. } => {
                tracing::info!(
                    query = text,
                    top_score,
                    "no match — fallback to general chat"
                );
            }
        }

        Ok(result)
    }

    /// Record user feedback for a skill match.
    ///
    /// Adjusts the skill's preference weight based on the feedback type.
    pub fn record_feedback(
        &self,
        user_input: &str,
        matched_skill: &str,
        score: f32,
        feedback_type: FeedbackType,
    ) -> f64 {
        let mut prefs = self.preferences.write();
        self.feedback
            .record(user_input, matched_skill, score, feedback_type, &mut prefs)
    }

    /// Record a correction: the user selected a non-top-1 candidate.
    ///
    /// During cold-start, this may trigger an automatic weight boost for the
    /// selected skill.
    pub fn record_correction(
        &self,
        user_input: &str,
        top_1_skill: &str,
        selected_skill: &str,
    ) -> Option<f64> {
        let mut prefs = self.preferences.write();
        self.cold_start
            .record_correction(user_input, top_1_skill, selected_skill, &mut prefs)
    }

    /// Get a clone of the current skill preferences.
    pub fn preferences(&self) -> SkillPreferences {
        self.preferences.read().clone()
    }

    /// Set the sensitivity level.
    pub fn set_sensitivity(&self, sensitivity: Sensitivity) {
        self.preferences.write().set_sensitivity(sensitivity);
    }

    /// Get the current sensitivity level.
    pub fn sensitivity(&self) -> Sensitivity {
        self.preferences.read().sensitivity()
    }

    /// Set a skill's preference weight.
    pub fn set_skill_weight(&self, skill_name: &str, weight: f64) {
        self.preferences.write().set_weight(skill_name, weight);
    }

    /// Get a skill's preference weight.
    pub fn get_skill_weight(&self, skill_name: &str) -> f64 {
        self.preferences.read().get_weight(skill_name)
    }

    /// Get all feedback records.
    pub fn feedback_records(&self) -> Vec<FeedbackRecord> {
        self.feedback.records()
    }

    /// Get all correction samples.
    pub fn corrections(&self) -> Vec<CorrectionSample> {
        self.cold_start.corrections()
    }

    /// Check if we're in the cold-start period.
    pub fn is_cold_start(&self) -> bool {
        self.cold_start.is_cold_start()
    }

    /// Get the current usage count.
    pub fn usage_count(&self) -> usize {
        self.cold_start.usage_count()
    }

    /// Get the number of skills in the index.
    pub fn index_size(&self) -> usize {
        self.index.len()
    }

    /// Get the embedding dimension.
    pub fn dimension(&self) -> Option<usize> {
        self.provider.dimension()
    }

    /// Get the top-K setting.
    pub fn top_k(&self) -> usize {
        self.top_k
    }
}

impl std::fmt::Debug for SemanticRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticRouter")
            .field("index_size", &self.index.len())
            .field("sensitivity", &self.sensitivity())
            .field("top_k", &self.top_k)
            .field("usage_count", &self.usage_count())
            .field("is_cold_start", &self.is_cold_start())
            .field("feedback_count", &self.feedback.len())
            .field("correction_count", &self.cold_start.correction_count())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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

    fn make_test_skills() -> Vec<Skill> {
        vec![
            make_skill("read_file", &["读取文件", "打开文件", "read file"]),
            make_skill("write_file", &["写入文件", "保存文件", "write file"]),
            make_skill("send_email", &["发送邮件", "发邮件", "send email"]),
            make_skill("search_web", &["搜索网络", "网页搜索", "search web"]),
            make_skill("run_command", &["执行命令", "运行命令", "run command"]),
        ]
    }

    fn make_router() -> SemanticRouter {
        let provider = Arc::new(MockEmbeddingProvider::new(128));
        SemanticRouter::new(provider)
    }

    #[tokio::test]
    async fn test_router_creation() {
        let router = make_router();
        assert_eq!(router.index_size(), 0);
        assert_eq!(router.sensitivity(), Sensitivity::Balanced);
        assert!(router.is_cold_start());
        assert_eq!(router.usage_count(), 0);
    }

    #[tokio::test]
    async fn test_router_rebuild_and_route() {
        let router = make_router();
        let skills = make_test_skills();
        router.rebuild_index_from_skills(&skills).await.unwrap();

        assert_eq!(router.index_size(), 5);

        let result = router.route("读取文件").await.unwrap();
        // Should be a direct match or candidates
        match &result {
            RouteResult::DirectMatch { skill, .. } => {
                assert_eq!(skill.name, "read_file");
            }
            RouteResult::Candidates { skills, .. } => {
                assert_eq!(skills[0].skill.name, "read_file");
            }
            RouteResult::NoMatch { .. } => {
                panic!("should match read_file");
            }
        }
    }

    #[tokio::test]
    async fn test_router_no_match() {
        let router = make_router();
        let skills = make_test_skills();
        router.rebuild_index_from_skills(&skills).await.unwrap();

        // Unrelated text
        let result = router.route("zzz xxx qqq").await.unwrap();
        assert!(matches!(result, RouteResult::NoMatch { .. }));
    }

    #[tokio::test]
    async fn test_router_empty_index() {
        let router = make_router();

        let result = router.route("读取文件").await.unwrap();
        assert!(matches!(result, RouteResult::NoMatch { .. }));
    }

    #[tokio::test]
    async fn test_router_feedback() {
        let router = make_router();
        let skills = make_test_skills();
        router.rebuild_index_from_skills(&skills).await.unwrap();

        // Route first
        let _ = router.route("读取文件").await.unwrap();

        // Record feedback
        let delta = router.record_feedback(
            "读取文件",
            "read_file",
            0.85,
            FeedbackType::ExplicitPositive,
        );

        assert!((delta - 0.05).abs() < 1e-10);
        assert!((router.get_skill_weight("read_file") - 1.05).abs() < 1e-10);

        // Feedback should be recorded
        let records = router.feedback_records();
        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn test_router_correction() {
        let router = make_router();
        let skills = make_test_skills();
        router.rebuild_index_from_skills(&skills).await.unwrap();

        // Route
        let _ = router.route("读取文件").await.unwrap();

        // Record a correction (user chose write_file instead of read_file)
        let boost = router.record_correction("读取文件", "read_file", "write_file");

        // First correction — no boost yet
        assert!(boost.is_none());
        assert_eq!(router.corrections().len(), 1);
    }

    #[tokio::test]
    async fn test_router_sensitivity() {
        let router = make_router();

        router.set_sensitivity(Sensitivity::Aggressive);
        assert_eq!(router.sensitivity(), Sensitivity::Aggressive);

        router.set_sensitivity(Sensitivity::Conservative);
        assert_eq!(router.sensitivity(), Sensitivity::Conservative);
    }

    #[tokio::test]
    async fn test_router_weight_setting() {
        let router = make_router();

        router.set_skill_weight("read_file", 2.0);
        assert_eq!(router.get_skill_weight("read_file"), 2.0);

        router.set_skill_weight("write_file", 0.5);
        assert_eq!(router.get_skill_weight("write_file"), 0.5);
    }

    #[tokio::test]
    async fn test_router_weight_affects_ranking() {
        let router = make_router();
        let skills = make_test_skills();
        router.rebuild_index_from_skills(&skills).await.unwrap();

        // Give write_file a massive weight boost
        router.set_skill_weight("write_file", 5.0);
        router.set_skill_weight("read_file", 0.1);

        let result = router.route("读取文件").await.unwrap();

        let top_skill = match &result {
            RouteResult::DirectMatch { skill, .. } => skill.name.clone(),
            RouteResult::Candidates { skills, .. } => skills[0].skill.name.clone(),
            RouteResult::NoMatch { .. } => panic!("expected match"),
        };

        // With extreme weights, write_file should rank first
        assert_eq!(top_skill, "write_file");
    }

    #[tokio::test]
    async fn test_router_usage_count_increments() {
        let router = make_router();
        let skills = make_test_skills();
        router.rebuild_index_from_skills(&skills).await.unwrap();

        assert_eq!(router.usage_count(), 0);

        let _ = router.route("test").await.unwrap();
        assert_eq!(router.usage_count(), 1);

        let _ = router.route("test2").await.unwrap();
        assert_eq!(router.usage_count(), 2);
    }

    #[tokio::test]
    async fn test_router_with_config() {
        let provider = Arc::new(MockEmbeddingProvider::new(64));
        let router = SemanticRouter::with_config(provider, 5, Sensitivity::Aggressive);

        assert_eq!(router.top_k(), 5);
        assert_eq!(router.sensitivity(), Sensitivity::Aggressive);
    }

    #[tokio::test]
    async fn test_router_debug() {
        let router = make_router();
        let debug_str = format!("{router:?}");
        assert!(debug_str.contains("SemanticRouter"));
        assert!(debug_str.contains("Balanced"));
    }

    #[tokio::test]
    async fn test_disabled_skill_excluded() {
        let router = make_router();
        let mut skills = make_test_skills();

        // Disable read_file
        skills[0].enabled = false;

        router.rebuild_index_from_skills(&skills).await.unwrap();

        // Index should have 4 skills, not 5
        assert_eq!(router.index_size(), 4);

        // read_file should not be in the results
        let result = router.route("读取文件").await.unwrap();
        let names = match &result {
            RouteResult::DirectMatch { skill, .. } => vec![skill.name.clone()],
            RouteResult::Candidates { skills, .. } => {
                skills.iter().map(|s| s.skill.name.clone()).collect()
            }
            RouteResult::NoMatch { .. } => vec![],
        };

        assert!(
            !names.contains(&"read_file".to_string()),
            "disabled skill should not appear in results"
        );
    }
}
