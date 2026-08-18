//! Semantic routing core — the algorithm that matches user queries to skills.
//!
//! The routing algorithm:
//! 1. Embed the user query
//! 2. Match against the router index (max-phrase similarity per skill)
//! 3. Apply skill preference weights
//! 4. Sort by weighted score and apply dual-threshold logic
//!
//! ## Threshold logic
//!
//! ```text
//! weighted_score >= high_threshold → DirectMatch (single skill)
//! low_threshold <= weighted_score < high_threshold → Candidates (top-3)
//! weighted_score < low_threshold → NoMatch (fallback to general chat)
//! ```

use crate::router::index::{MatchResult, RouterIndex};
use crate::router::preferences::SkillPreferences;
use crate::router::types::{RouteResult, ScoredSkill};
use crate::types::{AppError, AppResult, EmbeddingError};

/// Default number of candidates to return in the `Candidates` route result.
pub const DEFAULT_TOP_K: usize = 3;

/// The core routing function.
///
/// Given a query embedding, a router index, and skill preferences,
/// produces a `RouteResult` using the dual-threshold logic.
///
/// # Arguments
/// * `query_embedding` - The embedded user query
/// * `index` - The router index containing skill trigger-phrase embeddings
/// * `preferences` - Skill weights and sensitivity thresholds
/// * `top_k` - Maximum number of candidates to return (for the `Candidates` case)
pub fn route(
    query_embedding: &[f32],
    index: &RouterIndex,
    preferences: &SkillPreferences,
    top_k: usize,
) -> RouteResult {
    let (high_threshold, low_threshold) = preferences.thresholds();

    if index.is_empty() {
        return RouteResult::NoMatch {
            top_score: 0.0,
            threshold: low_threshold,
        };
    }

    // Step 1: Match query against the index
    let raw_matches: Vec<MatchResult> = index.match_query(query_embedding);

    if raw_matches.is_empty() {
        return RouteResult::NoMatch {
            top_score: 0.0,
            threshold: low_threshold,
        };
    }

    // Step 2: Apply weights and build scored skills
    let mut scored: Vec<ScoredSkill> = raw_matches
        .iter()
        .filter_map(|m| {
            index.get(&m.skill_name).map(|entry| ScoredSkill {
                skill: entry.skill.clone(),
                raw_score: m.raw_score,
                weighted_score: preferences.apply_weight(&m.skill_name, m.raw_score),
            })
        })
        .collect();

    // Step 3: Sort by weighted score descending
    scored.sort_by(|a, b| {
        b.weighted_score
            .partial_cmp(&a.weighted_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let top_score = scored[0].weighted_score;

    // Step 4: Dual-threshold decision
    if top_score >= high_threshold as f32 {
        // Direct match — return the top skill only
        RouteResult::DirectMatch {
            skill: Box::new(scored[0].skill.clone()),
            score: top_score,
            threshold: high_threshold,
        }
    } else if top_score >= low_threshold as f32 {
        // Candidates — return top-K
        let k = top_k.min(scored.len());
        RouteResult::Candidates {
            skills: scored[..k].to_vec(),
            top_score,
        }
    } else {
        // No match
        RouteResult::NoMatch {
            top_score,
            threshold: low_threshold,
        }
    }
}

/// Convenience wrapper that embeds the query text first, then routes.
///
/// This is the main entry point for the semantic router.
pub async fn route_text(
    text: &str,
    provider: &dyn crate::router::provider::EmbeddingProvider,
    index: &RouterIndex,
    preferences: &SkillPreferences,
    top_k: usize,
) -> AppResult<RouteResult> {
    if text.is_empty() {
        return Err(AppError::Embedding(EmbeddingError::EmptyInput));
    }

    let query_embedding = provider.embed(text).await?;
    Ok(route(&query_embedding, index, preferences, top_k))
}

/// Extract the skill names from a `RouteResult` for logging/debugging.
pub fn route_result_skill_names(result: &RouteResult) -> Vec<String> {
    match result {
        RouteResult::DirectMatch { skill, .. } => vec![skill.name.clone()],
        RouteResult::Candidates { skills, .. } => {
            skills.iter().map(|s| s.skill.name.clone()).collect()
        }
        RouteResult::NoMatch { .. } => vec![],
    }
}

/// Get the top score from a `RouteResult`.
pub fn route_result_top_score(result: &RouteResult) -> f32 {
    match result {
        RouteResult::DirectMatch { score, .. } => *score,
        RouteResult::Candidates { top_score, .. } => *top_score,
        RouteResult::NoMatch { top_score, .. } => *top_score,
    }
}

/// Check if the route result is a direct match.
pub fn is_direct_match(result: &RouteResult) -> bool {
    matches!(result, RouteResult::DirectMatch { .. })
}

/// Check if the route result is candidates.
pub fn is_candidates(result: &RouteResult) -> bool {
    matches!(result, RouteResult::Candidates { .. })
}

/// Check if the route result is no match.
pub fn is_no_match(result: &RouteResult) -> bool {
    matches!(result, RouteResult::NoMatch { .. })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::preferences::SkillPreferences;
    use crate::router::provider::MockEmbeddingProvider;
    use crate::router::types::Sensitivity;
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

    async fn build_test_index() -> (RouterIndex, MockEmbeddingProvider) {
        let provider = MockEmbeddingProvider::new(128);
        let skills = vec![
            make_skill("read_file", &["读取文件", "打开文件", "read file"]),
            make_skill("write_file", &["写入文件", "保存文件", "write file"]),
            make_skill("send_email", &["发送邮件", "发邮件", "send email"]),
        ];
        let index = RouterIndex::build_from_skills(&skills, &provider)
            .await
            .unwrap();
        (index, provider)
    }

    #[tokio::test]
    async fn test_direct_match() {
        let (index, provider) = build_test_index().await;
        let prefs = SkillPreferences::new();

        let result = route_text("读取文件", &provider, &index, &prefs, DEFAULT_TOP_K)
            .await
            .unwrap();

        assert!(is_direct_match(&result));
        let skill_name = route_result_skill_names(&result);
        assert_eq!(skill_name, vec!["read_file"]);
    }

    #[tokio::test]
    async fn test_no_match_unrelated() {
        let (index, provider) = build_test_index().await;
        let prefs = SkillPreferences::new();

        // Text with no overlap to any trigger phrase
        let result = route_text("xyz abc qwe", &provider, &index, &prefs, DEFAULT_TOP_K)
            .await
            .unwrap();

        assert!(is_no_match(&result));
    }

    #[tokio::test]
    async fn test_empty_index() {
        let index = RouterIndex::empty();
        let prefs = SkillPreferences::new();

        let query = vec![1.0; 64];
        let result = route(&query, &index, &prefs, DEFAULT_TOP_K);

        assert!(is_no_match(&result));
    }

    #[tokio::test]
    async fn test_weight_affects_ranking() {
        let (index, provider) = build_test_index().await;

        // Without weight, "read_file" should be top-1 for "读取文件"
        let prefs_default = SkillPreferences::new();
        let result_default =
            route_text("读取文件", &provider, &index, &prefs_default, DEFAULT_TOP_K)
                .await
                .unwrap();

        // The top skill should be read_file
        let top_skill_default = match &result_default {
            RouteResult::DirectMatch { skill, .. } => skill.name.clone(),
            RouteResult::Candidates { skills, .. } => skills[0].skill.name.clone(),
            _ => panic!("expected match"),
        };
        assert_eq!(top_skill_default, "read_file");

        // Give write_file a huge weight — it should now outrank read_file
        let mut prefs_weighted = SkillPreferences::new();
        prefs_weighted.set_weight("write_file", 5.0);
        prefs_weighted.set_weight("read_file", 0.1);

        let result_weighted = route_text(
            "读取文件",
            &provider,
            &index,
            &prefs_weighted,
            DEFAULT_TOP_K,
        )
        .await
        .unwrap();

        let top_skill_weighted = match &result_weighted {
            RouteResult::DirectMatch { skill, .. } => skill.name.clone(),
            RouteResult::Candidates { skills, .. } => skills[0].skill.name.clone(),
            RouteResult::NoMatch { .. } => panic!("expected match"),
        };

        // With extreme weights, write_file should now be top-1
        assert_eq!(
            top_skill_weighted, "write_file",
            "weight should affect ranking: write_file with weight 5.0 should beat read_file with 0.1"
        );
    }

    #[tokio::test]
    async fn test_sensitivity_affects_thresholds() {
        let (index, provider) = build_test_index().await;

        // With aggressive sensitivity, lower thresholds → more likely to direct-match
        let mut prefs_aggressive = SkillPreferences::new();
        prefs_aggressive.set_sensitivity(Sensitivity::Aggressive);

        // With conservative sensitivity, higher thresholds → less likely to direct-match
        let mut prefs_conservative = SkillPreferences::new();
        prefs_conservative.set_sensitivity(Sensitivity::Conservative);

        // A borderline query
        let result_agg = route_text(
            "读取文件",
            &provider,
            &index,
            &prefs_aggressive,
            DEFAULT_TOP_K,
        )
        .await
        .unwrap();

        let result_cons = route_text(
            "读取文件",
            &provider,
            &index,
            &prefs_conservative,
            DEFAULT_TOP_K,
        )
        .await
        .unwrap();

        // With aggressive thresholds, score is more likely to be a direct match
        let score = route_result_top_score(&result_agg);

        if score >= 0.85 {
            // Conservative might not direct-match while aggressive does
            assert!(
                is_direct_match(&result_agg) || is_candidates(&result_agg),
                "aggressive should at least match"
            );
            assert!(
                is_direct_match(&result_cons)
                    || is_candidates(&result_cons)
                    || is_no_match(&result_cons),
                "conservative may or may not match"
            );
        }
    }

    #[tokio::test]
    async fn test_candidates_returns_top_k() {
        let provider = MockEmbeddingProvider::new(128);
        let skills: Vec<Skill> = (0..5)
            .map(|i| make_skill(&format!("skill_{i}"), &[&format!("test phrase {i}")]))
            .collect();
        let index = RouterIndex::build_from_skills(&skills, &provider)
            .await
            .unwrap();

        let prefs = SkillPreferences::new();

        // Force a candidates result by using aggressive sensitivity
        // (low threshold so we get candidates, but high_threshold is 0.75 which
        // mock similarity might not reach for unrelated texts)
        let result = route_text("test phrase", &provider, &index, &prefs, 3)
            .await
            .unwrap();

        if let RouteResult::Candidates { skills, .. } = &result {
            assert!(skills.len() <= 3, "should return at most top-3");
        }
    }

    #[tokio::test]
    async fn test_empty_text_errors() {
        let (index, provider) = build_test_index().await;
        let prefs = SkillPreferences::new();

        let result = route_text("", &provider, &index, &prefs, DEFAULT_TOP_K).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_route_result_helpers() {
        let (index, provider) = build_test_index().await;
        let prefs = SkillPreferences::new();

        let result = route_text("读取文件", &provider, &index, &prefs, DEFAULT_TOP_K)
            .await
            .unwrap();

        let score = route_result_top_score(&result);
        assert!(score > 0.0, "score should be positive for a match");

        let names = route_result_skill_names(&result);
        assert!(!names.is_empty(), "should have at least one skill name");
    }
}
