//! Guardian Layer IPC commands.
//!
//! These functions are designed to be annotated with `#[tauri::command]`
//! once the Tauri runtime is integrated. For now they are plain async
//! functions that can be called from Rust or wrapped by the frontend bridge.

use std::sync::Arc;

use serde_json::Value;

use crate::guardian::{
    Guardian, GuardianConfig, LayerResult, ValidationLogEntry, ValidationResult,
};
use crate::router::slot_filler::LlmProvider;
use crate::skill::schema::Skill;
use crate::types::GuardianResult;

/// Create a new Guardian with the given configuration.
pub fn create_guardian(config: GuardianConfig) -> Guardian {
    Guardian::new(config)
}

/// Create a Guardian with default configuration.
pub fn create_default_guardian() -> Guardian {
    Guardian::with_defaults()
}

/// Create a Guardian with a custom semantic checker (for L3).
pub fn create_guardian_with_semantic_checker(
    config: GuardianConfig,
    checker: Arc<dyn crate::guardian::SemanticChecker>,
) -> Guardian {
    Guardian::with_semantic_checker(config, checker)
}

/// Validate Skill output with LLM-based retry.
///
/// This is the main entry point for production validation. It runs the
/// four-layer pipeline and retries on L1/L2 failure using the LLM provider
/// to generate corrected output.
pub async fn validate_output(
    guardian: &Guardian,
    skill: &Skill,
    raw_output: &str,
    provider: &dyn LlmProvider,
) -> GuardianResult<ValidationResult> {
    guardian
        .validate_with_retry(skill, raw_output, provider)
        .await
}

/// Validate Skill output without retry (single pass).
///
/// Runs the four-layer validation once. Does not call the LLM.
pub async fn validate_output_once(
    guardian: &Guardian,
    skill: &Skill,
    raw_output: &str,
) -> GuardianResult<ValidationResult> {
    guardian.validate_once(skill, raw_output).await
}

/// Check if a validation result passed all checks.
pub fn is_passed(result: &ValidationResult) -> bool {
    result.is_passed()
}

/// Check if a validation result was blocked by security (L4).
pub fn is_blocked(result: &ValidationResult) -> bool {
    result.is_blocked()
}

/// Get the validated output from a successful result.
///
/// Returns `None` if the result did not pass.
pub fn get_output(result: &ValidationResult) -> Option<&Value> {
    result.output.as_ref()
}

/// Get warnings from a validation result.
pub fn get_warnings(result: &ValidationResult) -> &[String] {
    &result.warnings
}

/// Get the number of validation attempts made.
pub fn get_attempts(result: &ValidationResult) -> usize {
    result.attempts
}

/// Check if the output was truncated due to size limits.
pub fn was_truncated(result: &ValidationResult) -> bool {
    result.was_truncated
}

/// Get the result for a specific validation layer.
///
/// Valid layer names: "L1", "L2", "L3", "L4" (case-insensitive).
/// Also accepts: "format", "schema", "semantic", "security".
pub fn get_layer_result<'a>(result: &'a ValidationResult, layer: &str) -> Option<&'a LayerResult> {
    match layer.to_lowercase().as_str() {
        "l1" | "format" => Some(&result.l1_format),
        "l2" | "schema" => Some(&result.l2_schema),
        "l3" | "semantic" => Some(&result.l3_semantic),
        "l4" | "security" => Some(&result.l4_security),
        _ => None,
    }
}

/// Get all validation log entries.
pub fn get_logs(guardian: &Guardian) -> Vec<ValidationLogEntry> {
    guardian.logs()
}

/// Clear all validation logs.
pub fn clear_logs(guardian: &Guardian) {
    guardian.clear_logs();
}

/// Get the Guardian's configuration.
pub fn get_config(guardian: &Guardian) -> &GuardianConfig {
    guardian.config()
}

/// Create a default GuardianConfig.
pub fn default_config() -> GuardianConfig {
    GuardianConfig::default()
}

/// Create a GuardianConfig with custom settings.
pub fn custom_config(
    l1: bool,
    l2: bool,
    l3: bool,
    l4: bool,
    max_retries: usize,
    max_output_size: usize,
    enable_truncation: bool,
) -> GuardianConfig {
    GuardianConfig {
        l1_format_check: l1,
        l2_schema_check: l2,
        l3_semantic_check: l3,
        l4_security_check: l4,
        max_retries,
        max_output_size,
        enable_truncation,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::slot_filler::MockLlmProvider;
    use crate::skill::schema::{Skill, SkillRuntime, SkillRuntimeType};
    use serde_json::Value;
    use std::path::PathBuf;

    fn make_skill(name: &str, output_schema: Value) -> Skill {
        Skill {
            mcp: None,
            schema_version: "1.0".to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Test skill {name}"),
            category: "test".to_string(),
            trigger_phrases: vec!["test".to_string()],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Python,
                entry: "script.py".to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({}),
            output_schema,
            permissions: Default::default(),
            tags: vec![],
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from(format!("/skills/{name}")),
        }
    }

    fn schema_with_content() -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": { "type": "string" }
            }
        })
    }

    #[tokio::test]
    async fn test_validate_output_success() {
        let guardian = create_default_guardian();
        let skill = make_skill("read_file", schema_with_content());
        let provider = MockLlmProvider::single(r#"{"content": "hello"}"#.to_string());

        let result = validate_output(&guardian, &skill, r#"{"content": "hello"}"#, &provider)
            .await
            .unwrap();

        assert!(is_passed(&result));
        assert!(!is_blocked(&result));
        assert_eq!(get_attempts(&result), 1);
        assert!(!was_truncated(&result));

        let output = get_output(&result).unwrap();
        assert_eq!(output["content"], "hello");
    }

    #[tokio::test]
    async fn test_validate_output_retry() {
        let guardian = create_default_guardian();
        let skill = make_skill("read_file", schema_with_content());
        let provider = MockLlmProvider::new(vec![r#"{"content": "fixed"}"#.to_string()]);

        let result = validate_output(&guardian, &skill, "bad json", &provider)
            .await
            .unwrap();

        assert!(is_passed(&result));
        assert_eq!(get_attempts(&result), 2);
    }

    #[tokio::test]
    async fn test_validate_output_once() {
        let guardian = create_default_guardian();
        let skill = make_skill("read_file", schema_with_content());

        let result = validate_output_once(&guardian, &skill, r#"{"content": "hello"}"#)
            .await
            .unwrap();

        assert!(is_passed(&result));
    }

    #[tokio::test]
    async fn test_validate_output_security_block() {
        let guardian = create_default_guardian();
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));
        let provider = MockLlmProvider::single(r#"{"clean": "yes"}"#.to_string());

        let output = r#"{"content": "sk-abcdefghijklmnopqrstuvwxyz1234567890"}"#;
        let result = validate_output(&guardian, &skill, output, &provider).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_layer_result() {
        let guardian = create_default_guardian();
        let skill = make_skill("read_file", schema_with_content());

        let result = validate_output_once(&guardian, &skill, r#"{"content": "hello"}"#)
            .await
            .unwrap();

        let l1 = get_layer_result(&result, "L1").unwrap();
        assert!(matches!(l1, LayerResult::Passed));

        let l2 = get_layer_result(&result, "format").unwrap();
        assert!(matches!(l2, LayerResult::Passed));

        assert!(get_layer_result(&result, "invalid").is_none());
    }

    #[tokio::test]
    async fn test_logs_command() {
        let guardian = create_default_guardian();
        let skill = make_skill("read_file", schema_with_content());

        let _ = validate_output_once(&guardian, &skill, r#"{"content": "hello"}"#).await;

        let logs = get_logs(&guardian);
        assert_eq!(logs.len(), 1);
        assert!(logs[0].passed);

        clear_logs(&guardian);
        assert!(get_logs(&guardian).is_empty());
    }

    #[tokio::test]
    async fn test_warnings_and_truncation() {
        let config = custom_config(true, true, false, true, 2, 10, true);
        let guardian = create_guardian(config);
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));

        let long_output = r#"{"content": "this is a very long output that exceeds 10 bytes"}"#;
        let result = validate_output_once(&guardian, &skill, long_output).await;

        // Truncation at 10 bytes breaks JSON → L1 fails (expected behavior).
        assert!(result.is_err());

        // But truncation warning should be in the logs.
        let logs = get_logs(&guardian);
        assert_eq!(logs.len(), 1);
        assert!(!logs[0].warnings.is_empty());
    }

    #[test]
    fn test_default_config_command() {
        let config = default_config();
        assert!(config.l1_format_check);
        assert!(config.l2_schema_check);
        assert!(config.l4_security_check);
    }

    #[test]
    fn test_custom_config_command() {
        let config = custom_config(false, false, true, false, 5, 2048, false);
        assert!(!config.l1_format_check);
        assert!(!config.l2_schema_check);
        assert!(config.l3_semantic_check);
        assert!(!config.l4_security_check);
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.max_output_size, 2048);
        assert!(!config.enable_truncation);
    }

    #[test]
    fn test_get_config_command() {
        let guardian = create_default_guardian();
        let config = get_config(&guardian);
        assert_eq!(config.max_retries, 2);
    }
}
