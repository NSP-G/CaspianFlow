//! Router IPC commands.
//!
//! These functions are designed to be annotated with `#[tauri::command]`
//! once the Tauri runtime is integrated. For now they are plain async
//! functions that can be called from Rust or wrapped by the frontend bridge.

use crate::router::{
    CorrectionSample, FeedbackRecord, FeedbackType, RouteResult, SemanticRouter, Sensitivity,
    SkillPreferences,
};
use crate::skill::registry::SkillRegistry;
use crate::types::AppResult;

/// Route a user query to the best-matching skill(s).
pub async fn route_query(router: &SemanticRouter, text: &str) -> AppResult<RouteResult> {
    router.route(text).await
}

/// Rebuild the router index from a skill registry.
pub async fn rebuild_index(router: &SemanticRouter, registry: &SkillRegistry) -> AppResult<()> {
    router.rebuild_index(registry).await
}

/// Record user feedback for a skill match.
pub fn record_feedback(
    router: &SemanticRouter,
    user_input: &str,
    matched_skill: &str,
    score: f32,
    feedback_type: FeedbackType,
) -> f64 {
    router.record_feedback(user_input, matched_skill, score, feedback_type)
}

/// Record a correction (user selected a non-top-1 candidate).
pub fn record_correction(
    router: &SemanticRouter,
    user_input: &str,
    top_1_skill: &str,
    selected_skill: &str,
) -> Option<f64> {
    router.record_correction(user_input, top_1_skill, selected_skill)
}

/// Set the sensitivity level.
pub fn set_sensitivity(router: &SemanticRouter, sensitivity: Sensitivity) {
    router.set_sensitivity(sensitivity);
}

/// Get the current sensitivity level.
pub fn get_sensitivity(router: &SemanticRouter) -> Sensitivity {
    router.sensitivity()
}

/// Set a skill's preference weight.
pub fn set_skill_weight(router: &SemanticRouter, skill_name: &str, weight: f64) {
    router.set_skill_weight(skill_name, weight);
}

/// Get a skill's preference weight.
pub fn get_skill_weight(router: &SemanticRouter, skill_name: &str) -> f64 {
    router.get_skill_weight(skill_name)
}

/// Get all skill preferences.
pub fn get_preferences(router: &SemanticRouter) -> SkillPreferences {
    router.preferences()
}

/// Get all feedback records.
pub fn get_feedback_records(router: &SemanticRouter) -> Vec<FeedbackRecord> {
    router.feedback_records()
}

/// Get all correction samples.
pub fn get_corrections(router: &SemanticRouter) -> Vec<CorrectionSample> {
    router.corrections()
}

/// Check if the router is in cold-start mode.
pub fn is_cold_start(router: &SemanticRouter) -> bool {
    router.is_cold_start()
}

/// Get the router's current usage count.
pub fn get_usage_count(router: &SemanticRouter) -> usize {
    router.usage_count()
}

/// Get the number of skills in the index.
pub fn get_index_size(router: &SemanticRouter) -> usize {
    router.index_size()
}

/// Get the embedding dimension.
pub fn get_dimension(router: &SemanticRouter) -> Option<usize> {
    router.dimension()
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
    use std::sync::Arc;

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

    fn make_router() -> SemanticRouter {
        let provider = Arc::new(MockEmbeddingProvider::new(128));
        SemanticRouter::new(provider)
    }

    #[tokio::test]
    async fn test_route_query() {
        let router = make_router();
        let skills = vec![
            make_skill("read_file", &["读取文件", "read file"]),
            make_skill("write_file", &["写入文件", "write file"]),
        ];
        router.rebuild_index_from_skills(&skills).await.unwrap();

        let result = route_query(&router, "读取文件").await.unwrap();
        match &result {
            RouteResult::DirectMatch { skill, .. } => {
                assert_eq!(skill.name, "read_file");
            }
            RouteResult::Candidates { skills, .. } => {
                assert_eq!(skills[0].skill.name, "read_file");
            }
            RouteResult::NoMatch { .. } => panic!("should match"),
        }
    }

    #[tokio::test]
    async fn test_route_query_empty() {
        let router = make_router();
        let result = route_query(&router, "").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_feedback_command() {
        let router = make_router();

        let delta = record_feedback(
            &router,
            "test",
            "read_file",
            0.8,
            FeedbackType::ExplicitPositive,
        );

        assert!((delta - 0.05).abs() < 1e-10);

        let records = get_feedback_records(&router);
        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn test_correction_command() {
        let router = make_router();

        record_correction(&router, "test", "read_file", "write_file");

        let corrections = get_corrections(&router);
        assert_eq!(corrections.len(), 1);
    }

    #[tokio::test]
    async fn test_sensitivity_commands() {
        let router = make_router();

        set_sensitivity(&router, Sensitivity::Aggressive);
        assert_eq!(get_sensitivity(&router), Sensitivity::Aggressive);

        set_sensitivity(&router, Sensitivity::Conservative);
        assert_eq!(get_sensitivity(&router), Sensitivity::Conservative);
    }

    #[tokio::test]
    async fn test_weight_commands() {
        let router = make_router();

        set_skill_weight(&router, "read_file", 2.0);
        assert_eq!(get_skill_weight(&router, "read_file"), 2.0);
    }

    #[tokio::test]
    async fn test_preferences_command() {
        let router = make_router();
        set_skill_weight(&router, "skill_a", 1.5);
        set_skill_weight(&router, "skill_b", 0.8);

        let prefs = get_preferences(&router);
        assert_eq!(prefs.len(), 2);
        assert_eq!(prefs.get_weight("skill_a"), 1.5);
        assert_eq!(prefs.get_weight("skill_b"), 0.8);
    }

    #[tokio::test]
    async fn test_cold_start_commands() {
        let router = make_router();

        assert!(is_cold_start(&router));
        assert_eq!(get_usage_count(&router), 0);

        // Route once to increment usage
        let _ = route_query(&router, "test").await;

        assert_eq!(get_usage_count(&router), 1);
    }

    #[tokio::test]
    async fn test_index_size_command() {
        let router = make_router();
        assert_eq!(get_index_size(&router), 0);

        let skills = vec![make_skill("test", &["test"])];
        router.rebuild_index_from_skills(&skills).await.unwrap();

        assert_eq!(get_index_size(&router), 1);
    }

    #[tokio::test]
    async fn test_dimension_command() {
        let router = make_router();
        assert_eq!(get_dimension(&router), Some(128));
    }
}
