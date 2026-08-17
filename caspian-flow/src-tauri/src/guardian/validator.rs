//! Guardian validator — orchestrates the four-layer validation pipeline.
//!
//! ## Design decision: zero-dependency schema validation
//!
//! The manual spec suggests using the `jsonschema` crate for L2 Schema
//! validation. However, P13 (Slot Filler) already implemented a lightweight,
//! zero-dependency JSON Schema validator (`validate_against_schema`) that
//! covers 11 common keywords. We reuse that validator here to:
//!
//! 1. Maintain architectural consistency (same validator for input + output)
//! 2. Honor the "zero dependencies" Caspian philosophy
//! 3. Avoid version conflicts from adding a new crate
//!
//! If future Skills require advanced JSON Schema features (anyOf, oneOf,
//! $ref), the validator can be extended in-place or the `jsonschema` crate
//! can be introduced at that time as a targeted addition.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::router::slot_filler::{apply_defaults, extract_json, validate_against_schema};
use crate::skill::schema::Skill;
use crate::types::{GuardianError, GuardianResult};

use super::security::SecurityChecker;

// ---------------------------------------------------------------------------
// Validation result types
// ---------------------------------------------------------------------------

/// The outcome of a single validation layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayerResult {
    /// The layer passed validation.
    Passed,
    /// The layer failed with specific errors.
    Failed { errors: Vec<String> },
    /// The layer was skipped (disabled in config).
    Skipped,
}

/// The complete validation result across all layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// L1 format check result.
    pub l1_format: LayerResult,
    /// L2 schema check result.
    pub l2_schema: LayerResult,
    /// L3 semantic check result.
    pub l3_semantic: LayerResult,
    /// L4 security check result.
    pub l4_security: LayerResult,
    /// The validated output (present if validation passed).
    pub output: Option<Value>,
    /// Warnings (non-blocking issues from L3 or size checks).
    pub warnings: Vec<String>,
    /// Number of retry attempts made (0 = first try succeeded).
    pub attempts: usize,
    /// Whether the output was truncated.
    pub was_truncated: bool,
}

impl ValidationResult {
    /// Check if all required layers passed.
    pub fn is_passed(&self) -> bool {
        let l1_ok = matches!(self.l1_format, LayerResult::Passed | LayerResult::Skipped);
        let l2_ok = matches!(self.l2_schema, LayerResult::Passed | LayerResult::Skipped);
        let l4_ok = matches!(self.l4_security, LayerResult::Passed | LayerResult::Skipped);
        // L3 is optional — warnings don't block
        l1_ok && l2_ok && l4_ok && self.output.is_some()
    }

    /// Check if validation was blocked by security (L4).
    pub fn is_blocked(&self) -> bool {
        matches!(self.l4_security, LayerResult::Failed { .. })
    }
}

// ---------------------------------------------------------------------------
// Semantic checker trait (L3 — stub for now)
// ---------------------------------------------------------------------------

/// Trait for semantic output checking (L3).
///
/// This is an optional layer that evaluates whether the output content
/// is semantically reasonable. The default implementation is a no-op.
/// When P23 (Model Adapter) is ready, an LLM-backed implementation can
/// be provided.
#[async_trait::async_trait]
pub trait SemanticChecker: Send + Sync {
    /// Check if the output is semantically reasonable.
    ///
    /// Returns a list of warnings. Empty list = no issues.
    async fn check(&self, skill: &Skill, output: &Value) -> Vec<String>;
}

/// Default implementation: no semantic checking.
pub struct NoopSemanticChecker;

#[async_trait::async_trait]
impl SemanticChecker for NoopSemanticChecker {
    async fn check(&self, _skill: &Skill, _output: &Value) -> Vec<String> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Guardian validator
// ---------------------------------------------------------------------------

/// The guardian validator — runs the four-layer validation pipeline.
///
/// This struct holds the security checker and semantic checker, and
/// orchestrates the validation flow. The retry logic is handled by
/// the `Guardian` struct in `mod.rs`.
pub struct GuardianValidator {
    security_checker: SecurityChecker,
    semantic_checker: Arc<dyn SemanticChecker>,
}

impl GuardianValidator {
    /// Create a new validator with default settings.
    pub fn new() -> Self {
        Self {
            security_checker: SecurityChecker::new(),
            semantic_checker: Arc::new(NoopSemanticChecker),
        }
    }

    /// Create a validator with a custom security checker.
    pub fn with_security_checker(mut self, checker: SecurityChecker) -> Self {
        self.security_checker = checker;
        self
    }

    /// Create a validator with a custom semantic checker.
    pub fn with_semantic_checker(mut self, checker: Arc<dyn SemanticChecker>) -> Self {
        self.semantic_checker = checker;
        self
    }

    /// Run a single validation pass (no retry) on the raw output.
    ///
    /// This is the core validation pipeline. The retry logic in `Guardian`
    /// calls this method and decides whether to retry based on the result.
    pub async fn validate_once(
        &self,
        skill: &Skill,
        raw_output: &str,
        config: &super::GuardianConfig,
    ) -> GuardianResult<ValidationResult> {
        let mut warnings = Vec::new();

        // --- Size check (always runs, even if truncation is disabled) ---
        let (output_str, size_warnings) = self.security_checker.check_size(raw_output);
        warnings.extend(size_warnings);
        let was_truncated = output_str.len() < raw_output.len();

        // --- L1: Format check ---
        let l1_result = if config.l1_format_check {
            match extract_json(&output_str) {
                Ok(value) => {
                    if value.is_object() {
                        LayerResult::Passed
                    } else {
                        LayerResult::Failed {
                            errors: vec![format!(
                                "output is valid JSON but not an object (got {})",
                                json_type_name(&value)
                            )],
                        }
                    }
                }
                Err(e) => LayerResult::Failed {
                    errors: vec![e.to_string()],
                },
            }
        } else {
            LayerResult::Skipped
        };

        // If L1 failed, we can't do L2/L3/L4 — return early with the failure.
        // The Guardian orchestrator (in mod.rs) decides whether to retry.
        if matches!(&l1_result, LayerResult::Failed { .. }) {
            return Ok(ValidationResult {
                l1_format: l1_result,
                l2_schema: LayerResult::Skipped,
                l3_semantic: LayerResult::Skipped,
                l4_security: LayerResult::Skipped,
                output: None,
                warnings,
                attempts: 0,
                was_truncated,
            });
        }

        // Parse the JSON (we know it's valid from L1)
        let mut parsed =
            extract_json(&output_str).map_err(|e| GuardianError::FormatCheckFailed {
                raw_output: e.to_string(),
            })?;

        // --- L2: Schema check ---
        let l2_result = if config.l2_schema_check {
            if skill.output_schema.is_null()
                || skill
                    .output_schema
                    .as_object()
                    .map(|o| o.is_empty())
                    .unwrap_or(true)
            {
                // Empty schema → skip L2
                LayerResult::Skipped
            } else {
                let errors = validate_against_schema(&parsed, &skill.output_schema);
                if errors.is_empty() {
                    // Apply defaults before returning
                    apply_defaults(&mut parsed, &skill.output_schema);
                    LayerResult::Passed
                } else {
                    LayerResult::Failed {
                        errors: errors.iter().map(|e| e.to_string()).collect(),
                    }
                }
            }
        } else {
            LayerResult::Skipped
        };

        // If L2 failed, skip L3/L4 (output is not schema-valid)
        if let LayerResult::Failed { .. } = &l2_result {
            return Ok(ValidationResult {
                l1_format: l1_result,
                l2_schema: l2_result,
                l3_semantic: LayerResult::Skipped,
                l4_security: LayerResult::Skipped,
                output: None,
                warnings,
                attempts: 0,
                was_truncated,
            });
        }

        // --- L3: Semantic check (optional, non-blocking) ---
        let l3_result = if config.l3_semantic_check {
            let sem_warnings = self.semantic_checker.check(skill, &parsed).await;
            if sem_warnings.is_empty() {
                LayerResult::Passed
            } else {
                // L3 warnings don't block — mark as passed with warnings
                warnings.extend(sem_warnings);
                LayerResult::Passed
            }
        } else {
            LayerResult::Skipped
        };

        // --- L4: Security check ---
        let l4_result = if config.l4_security_check {
            // Check the raw output string for secrets/dangerous content
            let violations = self.security_checker.check(&output_str);
            if violations.is_empty() {
                LayerResult::Passed
            } else {
                LayerResult::Failed {
                    errors: violations.iter().map(|v| v.to_string()).collect(),
                }
            }
        } else {
            LayerResult::Skipped
        };

        // Determine if we have a valid output
        let output = if matches!(l4_result, LayerResult::Failed { .. }) {
            None // Security blocked — don't return output
        } else {
            Some(parsed)
        };

        Ok(ValidationResult {
            l1_format: l1_result,
            l2_schema: l2_result,
            l3_semantic: l3_result,
            l4_security: l4_result,
            output,
            warnings,
            attempts: 0,
            was_truncated,
        })
    }

    /// Generate a correction prompt for a failed validation.
    ///
    /// This is used by the retry mechanism to ask the LLM to fix the output.
    pub fn generate_correction_prompt(
        skill: &Skill,
        raw_output: &str,
        result: &ValidationResult,
    ) -> String {
        let mut errors = Vec::new();

        if let LayerResult::Failed { errors: e } = &result.l1_format {
            errors.extend(e.iter().cloned());
        }
        if let LayerResult::Failed { errors: e } = &result.l2_schema {
            errors.extend(e.iter().cloned());
        }

        let error_list = errors
            .iter()
            .enumerate()
            .map(|(i, e)| format!("  {}. {}", i + 1, e))
            .collect::<Vec<_>>()
            .join("\n");

        let schema_str = if skill.output_schema.is_null() {
            "{}".to_string()
        } else {
            serde_json::to_string_pretty(&skill.output_schema)
                .unwrap_or_else(|_| skill.output_schema.to_string())
        };

        format!(
            r#"Skill 输出的校验未通过，请修正后重新输出。

Skill: {name}
描述: {description}

期望的输出 Schema:
{schema}

上次的输出:
{raw_output}

校验错误:
{errors}

请根据以上错误修正输出，确保输出为符合 Schema 的 JSON 对象。只输出 JSON，不要包含其他文本。
输出:"#,
            name = skill.display_name,
            description = skill.description,
            schema = schema_str,
            raw_output = raw_output,
            errors = error_list,
        )
    }
}

impl Default for GuardianValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for GuardianValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardianValidator")
            .field("security_checker", &self.security_checker)
            .finish()
    }
}

/// Get the JSON type name of a value for error messages.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => {
            if value.is_i64() || value.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
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

    fn default_config() -> super::super::GuardianConfig {
        super::super::GuardianConfig::default()
    }

    #[tokio::test]
    async fn test_validate_clean_output() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": { "type": "string" }
            }
        });
        let skill = make_skill("read_file", schema);
        let validator = GuardianValidator::new();
        let output = r#"{"content": "hello world"}"#;

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(result.is_passed());
        assert!(matches!(result.l1_format, LayerResult::Passed));
        assert!(matches!(result.l2_schema, LayerResult::Passed));
        assert!(matches!(result.l4_security, LayerResult::Passed));
        assert!(result.output.is_some());
    }

    #[tokio::test]
    async fn test_validate_invalid_json() {
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));
        let validator = GuardianValidator::new();
        let output = "this is not json";

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(!result.is_passed());
        assert!(matches!(result.l1_format, LayerResult::Failed { .. }));
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn test_validate_schema_missing_field() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": { "type": "string" }
            }
        });
        let skill = make_skill("read_file", schema);
        let validator = GuardianValidator::new();
        let output = r#"{"other": "value"}"#;

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(!result.is_passed());
        assert!(matches!(result.l1_format, LayerResult::Passed));
        assert!(matches!(result.l2_schema, LayerResult::Failed { .. }));
    }

    #[tokio::test]
    async fn test_validate_schema_wrong_type() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" }
            }
        });
        let skill = make_skill("counter", schema);
        let validator = GuardianValidator::new();
        let output = r#"{"count": "not a number"}"#;

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(!result.is_passed());
        assert!(matches!(result.l2_schema, LayerResult::Failed { .. }));
    }

    #[tokio::test]
    async fn test_validate_security_blocked() {
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));
        let validator = GuardianValidator::new();
        let output = r#"{"content": "sk-abcdefghijklmnopqrstuvwxyz1234567890"}"#;

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(!result.is_passed());
        assert!(result.is_blocked());
        assert!(matches!(result.l4_security, LayerResult::Failed { .. }));
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn test_validate_l1_disabled() {
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));
        let validator = GuardianValidator::new();
        let config = super::super::GuardianConfig {
            l1_format_check: false,
            ..default_config()
        };

        let output = r#"{"content": "hello"}"#;
        let result = validator
            .validate_once(&skill, output, &config)
            .await
            .unwrap();

        assert!(matches!(result.l1_format, LayerResult::Skipped));
    }

    #[tokio::test]
    async fn test_validate_l2_disabled() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["missing_field"],
            "properties": {
                "missing_field": { "type": "string" }
            }
        });
        let skill = make_skill("read_file", schema);
        let validator = GuardianValidator::new();
        let config = super::super::GuardianConfig {
            l2_schema_check: false,
            ..default_config()
        };

        let output = r#"{"content": "hello"}"#;
        let result = validator
            .validate_once(&skill, output, &config)
            .await
            .unwrap();

        // L2 skipped → missing field not caught → passes
        assert!(matches!(result.l2_schema, LayerResult::Skipped));
        assert!(result.is_passed());
    }

    #[tokio::test]
    async fn test_validate_l4_disabled() {
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));
        let validator = GuardianValidator::new();
        let config = super::super::GuardianConfig {
            l4_security_check: false,
            ..default_config()
        };

        let output = r#"{"content": "sk-abcdefghijklmnopqrstuvwxyz1234567890"}"#;
        let result = validator
            .validate_once(&skill, output, &config)
            .await
            .unwrap();

        // L4 skipped → API key not caught → passes
        assert!(matches!(result.l4_security, LayerResult::Skipped));
        assert!(result.is_passed());
    }

    #[tokio::test]
    async fn test_validate_empty_schema_skips_l2() {
        let skill = make_skill("read_file", serde_json::json!({}));
        let validator = GuardianValidator::new();
        let output = r#"{"anything": true}"#;

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(matches!(result.l2_schema, LayerResult::Skipped));
        assert!(result.is_passed());
    }

    #[tokio::test]
    async fn test_validate_with_defaults() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": { "type": "string" },
                "encoding": { "type": "string", "default": "utf-8" }
            }
        });
        let skill = make_skill("read_file", schema);
        let validator = GuardianValidator::new();
        let output = r#"{"content": "hello"}"#;

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(result.is_passed());
        let out = result.output.unwrap();
        assert_eq!(out["content"], "hello");
        assert_eq!(out["encoding"], "utf-8"); // default filled
    }

    #[tokio::test]
    async fn test_validate_markdown_wrapped_json() {
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));
        let validator = GuardianValidator::new();
        let output = "```json\n{\"content\": \"hello\"}\n```";

        let result = validator
            .validate_once(&skill, output, &default_config())
            .await
            .unwrap();

        assert!(matches!(result.l1_format, LayerResult::Passed));
        assert!(result.is_passed());
    }

    #[test]
    fn test_generate_correction_prompt() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": { "type": "string" }
            }
        });
        let skill = make_skill("read_file", schema);
        let result = ValidationResult {
            l1_format: LayerResult::Passed,
            l2_schema: LayerResult::Failed {
                errors: vec!["[$root.content] required field is missing".to_string()],
            },
            l3_semantic: LayerResult::Skipped,
            l4_security: LayerResult::Skipped,
            output: None,
            warnings: vec![],
            attempts: 0,
            was_truncated: false,
        };

        let prompt =
            GuardianValidator::generate_correction_prompt(&skill, r#"{"other": "value"}"#, &result);

        assert!(prompt.contains("校验未通过"));
        assert!(prompt.contains("required field is missing"));
        assert!(prompt.contains("read_file"));
    }

    #[test]
    fn test_validation_result_is_passed() {
        let result = ValidationResult {
            l1_format: LayerResult::Passed,
            l2_schema: LayerResult::Passed,
            l3_semantic: LayerResult::Skipped,
            l4_security: LayerResult::Passed,
            output: Some(serde_json::json!({"ok": true})),
            warnings: vec![],
            attempts: 0,
            was_truncated: false,
        };
        assert!(result.is_passed());
        assert!(!result.is_blocked());
    }

    #[test]
    fn test_validation_result_is_blocked() {
        let result = ValidationResult {
            l1_format: LayerResult::Passed,
            l2_schema: LayerResult::Passed,
            l3_semantic: LayerResult::Skipped,
            l4_security: LayerResult::Failed {
                errors: vec!["api_key detected".to_string()],
            },
            output: None,
            warnings: vec![],
            attempts: 0,
            was_truncated: false,
        };
        assert!(!result.is_passed());
        assert!(result.is_blocked());
    }

    #[tokio::test]
    async fn test_noop_semantic_checker() {
        let checker = NoopSemanticChecker;
        let skill = make_skill("test", serde_json::json!({}));
        let warnings = checker.check(&skill, &serde_json::json!({})).await;
        assert!(warnings.is_empty());
    }
}
