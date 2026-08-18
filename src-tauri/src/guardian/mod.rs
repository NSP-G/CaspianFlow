//! Guardian Layer — four-layer output validation with retry mechanism.
//!
//! The Guardian sits between Skill execution and the user, ensuring that
//! all Skill output meets quality and safety standards before it is returned.
//!
//! ## Validation layers
//!
//! | Layer | Check | Failure handling |
//! |-------|-------|-----------------|
//! | L1 | Format (valid JSON?) | Retry with correction prompt |
//! | L2 | Schema (matches output_schema?) | Retry with correction prompt |
//! | L3 | Semantic (content reasonable?) | Warning only, non-blocking |
//! | L4 | Security (no secrets/dangerous content) | Block immediately, no retry |
//!
//! L1 + L2 are mandatory; L3 + L4 are configurable.
//!
//! ## Retry flow
//!
//! ```text
//! Skill output
//!     |
//!     v
//! validate_once (L1 -> L2 -> L3 -> L4)
//!     |
//!     +-- Passed --> return Ok(output)
//!     |
//!     +-- L4 blocked --> return Err(SecurityBlocked) [no retry]
//!     |
//!     +-- L1/L2 failed --> generate correction prompt
//!                              |
//!                              v
//!                         LLM generates corrected output
//!                              |
//!                              v
//!                         retry (up to max_retries)
//!                              |
//!                              +-- still fails --> return Err(MaxRetriesExceeded)
//! ```

pub mod security;
pub mod validator;

pub use security::{SecurityChecker, SecurityViolation, DEFAULT_MAX_OUTPUT_SIZE};
pub use validator::{
    GuardianValidator, LayerResult, NoopSemanticChecker, SemanticChecker, ValidationResult,
};

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::router::slot_filler::LlmProvider;
use crate::skill::schema::Skill;
use crate::types::{GuardianError, GuardianResult};

// ---------------------------------------------------------------------------
// Guardian configuration
// ---------------------------------------------------------------------------

/// Configuration for the Guardian validation pipeline.
///
/// Controls which validation layers are active and the retry behavior.
/// L1 and L2 are mandatory (default: true) and should rarely be disabled.
/// L3 (semantic) and L4 (security) are optional but recommended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianConfig {
    /// L1: Check if output is valid JSON (default: true, mandatory).
    pub l1_format_check: bool,
    /// L2: Validate output against output_schema (default: true, mandatory).
    pub l2_schema_check: bool,
    /// L3: Semantic check via LLM (default: false, optional).
    pub l3_semantic_check: bool,
    /// L4: Security check for secrets/dangerous content (default: true).
    pub l4_security_check: bool,
    /// Maximum retry attempts on L1/L2 failure (default: 2, total 3 attempts).
    pub max_retries: usize,
    /// Maximum output size in bytes (default: 1 MB).
    pub max_output_size: usize,
    /// Whether to truncate oversized output (default: true).
    pub enable_truncation: bool,
}

impl Default for GuardianConfig {
    fn default() -> Self {
        Self {
            l1_format_check: true,
            l2_schema_check: true,
            l3_semantic_check: false,
            l4_security_check: true,
            max_retries: 2,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
            enable_truncation: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation log
// ---------------------------------------------------------------------------

/// A log entry for a single validation attempt.
///
/// Each call to `validate_once` or `validate_with_retry` produces one or
/// more log entries. These are useful for debugging validation failures
/// and analyzing output quality over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationLogEntry {
    /// Attempt number (1-based).
    pub attempt: usize,
    /// Which layer caused the failure (None if passed).
    pub failed_layer: Option<String>,
    /// Error messages from the failed layer.
    pub errors: Vec<String>,
    /// Warnings generated during this attempt.
    pub warnings: Vec<String>,
    /// Whether this attempt passed all checks.
    pub passed: bool,
    /// Unix timestamp (seconds).
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Guardian — the orchestrator
// ---------------------------------------------------------------------------

/// The Guardian — orchestrates the four-layer validation pipeline with retry.
///
/// The Guardian owns a [`GuardianValidator`] and a [`GuardianConfig`].
/// It calls `validate_once` and decides whether to retry based on the result:
///
/// - **L1/L2 failure** -> generate correction prompt, retry (up to `max_retries`)
/// - **L4 failure** -> immediate block, no retry
/// - **L3 warnings** -> non-blocking, logged but don't trigger retry
///
/// ## Usage
///
/// ```no_run
/// # use caspian_flow::guardian::{Guardian, GuardianConfig};
/// # use caspian_flow::skill::schema::Skill;
/// # use caspian_flow::router::slot_filler::MockLlmProvider;
/// # async fn example(skill: &Skill) {
/// let guardian = Guardian::with_defaults();
/// let provider = MockLlmProvider::single(r#"{"content": "hello"}"#.to_string());
///
/// let result = guardian
///     .validate_with_retry(skill, r#"{"content": "hello"}"#, &provider)
///     .await;
/// # }
/// ```
pub struct Guardian {
    /// The validator that runs the four-layer pipeline.
    validator: GuardianValidator,
    /// Configuration for validation layers and retry.
    config: GuardianConfig,
    /// Validation log entries (one per attempt).
    logs: parking_lot::Mutex<Vec<ValidationLogEntry>>,
}

impl Guardian {
    /// Create a new Guardian with the given configuration.
    ///
    /// The security checker is configured from the config's `max_output_size`
    /// and `enable_truncation` settings.
    pub fn new(config: GuardianConfig) -> Self {
        let security_checker = build_security_checker(&config);
        let validator = GuardianValidator::new().with_security_checker(security_checker);

        Self {
            validator,
            config,
            logs: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Create a Guardian with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(GuardianConfig::default())
    }

    /// Create a Guardian with a custom semantic checker (for L3).
    ///
    /// The security checker is still configured from the config.
    pub fn with_semantic_checker(
        config: GuardianConfig,
        checker: Arc<dyn SemanticChecker>,
    ) -> Self {
        let security_checker = build_security_checker(&config);
        let validator = GuardianValidator::new()
            .with_security_checker(security_checker)
            .with_semantic_checker(checker);

        Self {
            validator,
            config,
            logs: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Get the current configuration.
    pub fn config(&self) -> &GuardianConfig {
        &self.config
    }

    /// Get the validation log entries.
    pub fn logs(&self) -> Vec<ValidationLogEntry> {
        self.logs.lock().clone()
    }

    /// Clear the validation log.
    pub fn clear_logs(&self) {
        self.logs.lock().clear();
    }

    // -----------------------------------------------------------------------
    // Single-pass validation (no retry)
    // -----------------------------------------------------------------------

    /// Validate Skill output without retry (single pass).
    ///
    /// Runs the four-layer validation once and returns the result.
    /// Does not perform any LLM-based correction.
    ///
    /// Returns:
    /// - `Ok(ValidationResult)` if validation passed
    /// - `Err(FormatCheckFailed)` if L1 failed
    /// - `Err(SchemaValidationFailed)` if L2 failed
    /// - `Err(SecurityBlocked)` if L4 blocked the output
    pub async fn validate_once(
        &self,
        skill: &Skill,
        raw_output: &str,
    ) -> GuardianResult<ValidationResult> {
        let result = self
            .validator
            .validate_once(skill, raw_output, &self.config)
            .await?;

        self.log_attempt(1, &result);

        if result.is_passed() {
            return Ok(result);
        }

        // Determine the specific error
        if result.is_blocked() {
            let violations = layer_errors(&result.l4_security);
            return Err(GuardianError::SecurityBlocked { violations });
        }

        if matches!(result.l1_format, LayerResult::Failed { .. }) {
            return Err(GuardianError::FormatCheckFailed {
                raw_output: raw_output.to_string(),
            });
        }

        if matches!(result.l2_schema, LayerResult::Failed { .. }) {
            let errors = layer_errors(&result.l2_schema);
            return Err(GuardianError::SchemaValidationFailed {
                error_count: errors.len(),
                errors: errors.join("; "),
                raw_output: raw_output.to_string(),
            });
        }

        // Shouldn't reach here — but handle gracefully
        Err(GuardianError::MaxRetriesExceeded {
            max_retries: 0,
            attempts: 1,
            last_errors: collect_all_errors(&result),
            raw_output: raw_output.to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // Validation with LLM-based retry
    // -----------------------------------------------------------------------

    /// Validate Skill output with LLM-based retry.
    ///
    /// This is the main entry point for production use. It:
    /// 1. Runs the four-layer validation
    /// 2. If L1/L2 fail, generates a correction prompt and asks the LLM to fix
    /// 3. Retries up to `max_retries` times (total `max_retries + 1` attempts)
    /// 4. If L4 (security) fails, blocks immediately — no retry
    /// 5. Returns the validated output or an error
    ///
    /// The LLM provider is only called when a retry is needed. If the first
    /// validation passes, the provider is never invoked.
    pub async fn validate_with_retry(
        &self,
        skill: &Skill,
        raw_output: &str,
        provider: &dyn LlmProvider,
    ) -> GuardianResult<ValidationResult> {
        let max_attempts = self.config.max_retries + 1;
        let mut current_output = raw_output.to_string();
        let mut last_errors: Vec<String> = Vec::new();

        for attempt in 1..=max_attempts {
            tracing::info!(
                skill = %skill.name,
                attempt,
                max_attempts,
                "guardian validation attempt"
            );

            let result = self
                .validator
                .validate_once(skill, &current_output, &self.config)
                .await?;

            // Log the attempt
            self.log_attempt(attempt, &result);

            // Check if passed
            if result.is_passed() {
                tracing::info!(
                    skill = %skill.name,
                    attempt,
                    "guardian validation passed"
                );
                let mut result = result;
                result.attempts = attempt;
                return Ok(result);
            }

            // Check if blocked by security (L4) — no retry
            if result.is_blocked() {
                tracing::warn!(
                    skill = %skill.name,
                    attempt,
                    "guardian validation blocked by security"
                );
                let violations = layer_errors(&result.l4_security);
                return Err(GuardianError::SecurityBlocked { violations });
            }

            // Collect errors for the retry prompt and final error
            last_errors = collect_all_errors(&result);

            // If not the last attempt, generate correction prompt and retry
            if attempt < max_attempts {
                let correction_prompt =
                    GuardianValidator::generate_correction_prompt(skill, &current_output, &result);

                tracing::info!(
                    skill = %skill.name,
                    attempt,
                    "requesting LLM correction"
                );

                // Call the LLM — if it fails, propagate the error immediately
                current_output = provider.generate(&correction_prompt).await?;
            }
        }

        // All retries exhausted
        tracing::warn!(
            skill = %skill.name,
            "guardian validation failed after all {} attempts",
            max_attempts
        );

        Err(GuardianError::MaxRetriesExceeded {
            max_retries: self.config.max_retries,
            attempts: max_attempts,
            last_errors,
            raw_output: current_output,
        })
    }

    // -----------------------------------------------------------------------
    // Internal: logging
    // -----------------------------------------------------------------------

    /// Record a validation attempt in the log.
    fn log_attempt(&self, attempt: usize, result: &ValidationResult) {
        let failed_layer = if matches!(result.l1_format, LayerResult::Failed { .. }) {
            Some("L1_format".to_string())
        } else if matches!(result.l2_schema, LayerResult::Failed { .. }) {
            Some("L2_schema".to_string())
        } else if matches!(result.l4_security, LayerResult::Failed { .. }) {
            Some("L4_security".to_string())
        } else {
            None
        };

        let errors = collect_all_errors(result);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        self.logs.lock().push(ValidationLogEntry {
            attempt,
            failed_layer,
            errors,
            warnings: result.warnings.clone(),
            passed: result.is_passed(),
            timestamp,
        });
    }
}

impl std::fmt::Debug for Guardian {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guardian")
            .field("config", &self.config)
            .field("log_count", &self.logs.lock().len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Build a SecurityChecker from the GuardianConfig's size/truncation settings.
fn build_security_checker(config: &GuardianConfig) -> SecurityChecker {
    let mut checker = SecurityChecker::new().with_max_output_size(config.max_output_size);
    if !config.enable_truncation {
        checker = checker.with_truncation(false);
    }
    checker
}

/// Extract error messages from a LayerResult::Failed variant.
/// Returns an empty vec for Passed or Skipped.
fn layer_errors(layer: &LayerResult) -> Vec<String> {
    match layer {
        LayerResult::Failed { errors } => errors.clone(),
        _ => Vec::new(),
    }
}

/// Collect all error messages across L1, L2, and L4 from a ValidationResult.
fn collect_all_errors(result: &ValidationResult) -> Vec<String> {
    let mut errors = Vec::new();
    errors.extend(layer_errors(&result.l1_format));
    errors.extend(layer_errors(&result.l2_schema));
    errors.extend(layer_errors(&result.l4_security));
    errors
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

    // --- Test helpers ---

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

    // =====================
    // Config tests
    // =====================

    #[test]
    fn test_config_defaults() {
        let config = GuardianConfig::default();
        assert!(config.l1_format_check);
        assert!(config.l2_schema_check);
        assert!(!config.l3_semantic_check);
        assert!(config.l4_security_check);
        assert_eq!(config.max_retries, 2);
        assert_eq!(config.max_output_size, DEFAULT_MAX_OUTPUT_SIZE);
        assert!(config.enable_truncation);
    }

    #[test]
    fn test_config_serialization() {
        let config = GuardianConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: GuardianConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.l1_format_check, config.l1_format_check);
        assert_eq!(deserialized.max_retries, config.max_retries);
    }

    // =====================
    // validate_once tests
    // =====================

    #[tokio::test]
    async fn test_validate_once_clean() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());
        let output = r#"{"content": "hello world"}"#;

        let result = guardian.validate_once(&skill, output).await.unwrap();

        assert!(result.is_passed());
        assert!(result.output.is_some());
        assert_eq!(result.output.unwrap()["content"], "hello world");
    }

    #[tokio::test]
    async fn test_validate_once_l1_fail() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());

        let result = guardian.validate_once(&skill, "not json at all").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            GuardianError::FormatCheckFailed { .. } => {}
            other => panic!("expected FormatCheckFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_validate_once_l2_fail() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());

        let result = guardian
            .validate_once(&skill, r#"{"other": "value"}"#)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            GuardianError::SchemaValidationFailed { errors, .. } => {
                assert!(errors.contains("missing"));
            }
            other => panic!("expected SchemaValidationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_validate_once_l4_block() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));

        let output = r#"{"content": "sk-abcdefghijklmnopqrstuvwxyz1234567890"}"#;
        let result = guardian.validate_once(&skill, output).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            GuardianError::SecurityBlocked { violations } => {
                assert!(!violations.is_empty());
            }
            other => panic!("expected SecurityBlocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_validate_once_with_defaults() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": { "type": "string" },
                "encoding": { "type": "string", "default": "utf-8" }
            }
        });
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema);

        let result = guardian
            .validate_once(&skill, r#"{"content": "hello"}"#)
            .await
            .unwrap();

        assert!(result.is_passed());
        let out = result.output.unwrap();
        assert_eq!(out["content"], "hello");
        assert_eq!(out["encoding"], "utf-8");
    }

    #[tokio::test]
    async fn test_validate_once_empty_schema() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", serde_json::json!({}));

        let result = guardian
            .validate_once(&skill, r#"{"anything": true}"#)
            .await
            .unwrap();

        assert!(result.is_passed());
        // L2 should be skipped for empty schema
        assert!(matches!(result.l2_schema, LayerResult::Skipped));
    }

    // =====================
    // validate_with_retry tests
    // =====================

    #[tokio::test]
    async fn test_retry_success_first_try() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());
        let provider = MockLlmProvider::single(r#"{"content": "hello"}"#.to_string());

        let result = guardian
            .validate_with_retry(&skill, r#"{"content": "hello"}"#, &provider)
            .await
            .unwrap();

        assert!(result.is_passed());
        assert_eq!(result.attempts, 1);
        assert_eq!(provider.call_count(), 0); // LLM not called on first success
    }

    #[tokio::test]
    async fn test_retry_success_on_second_attempt() {
        // First output is invalid JSON, LLM corrects it on retry
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());
        let provider = MockLlmProvider::new(vec![r#"{"content": "corrected"}"#.to_string()]);

        let result = guardian
            .validate_with_retry(&skill, "not valid json", &provider)
            .await
            .unwrap();

        assert!(result.is_passed());
        assert_eq!(result.attempts, 2);
        assert_eq!(provider.call_count(), 1); // LLM called once for correction
        assert_eq!(result.output.unwrap()["content"], "corrected");
    }

    #[tokio::test]
    async fn test_retry_success_on_third_attempt() {
        // Two bad outputs, third is good
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());
        let provider = MockLlmProvider::new(vec![
            "still not json".to_string(),
            r#"{"content": "finally good"}"#.to_string(),
        ]);

        let result = guardian
            .validate_with_retry(&skill, "bad json", &provider)
            .await
            .unwrap();

        assert!(result.is_passed());
        assert_eq!(result.attempts, 3);
        assert_eq!(provider.call_count(), 2);
    }

    #[tokio::test]
    async fn test_retry_all_attempts_fail() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());
        let provider =
            MockLlmProvider::new(vec!["bad json 1".to_string(), "bad json 2".to_string()]);

        let result = guardian
            .validate_with_retry(&skill, "bad json 0", &provider)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            GuardianError::MaxRetriesExceeded {
                attempts,
                max_retries,
                ..
            } => {
                assert_eq!(attempts, 3);
                assert_eq!(max_retries, 2);
            }
            other => panic!("expected MaxRetriesExceeded, got {other:?}"),
        }
        assert_eq!(provider.call_count(), 2);
    }

    #[tokio::test]
    async fn test_retry_security_block_immediate() {
        // Output contains an API key — L4 should block immediately, no retry
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));
        let provider = MockLlmProvider::single(r#"{"clean": "output"}"#.to_string());

        let output = r#"{"content": "sk-abcdefghijklmnopqrstuvwxyz1234567890"}"#;
        let result = guardian
            .validate_with_retry(&skill, output, &provider)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GuardianError::SecurityBlocked { .. }
        ));
        // LLM should NOT be called — security block is immediate
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn test_retry_schema_correction() {
        // First output has wrong type, LLM corrects it
        let guardian = Guardian::with_defaults();
        let skill = make_skill(
            "counter",
            serde_json::json!({
                "type": "object",
                "required": ["count"],
                "properties": {
                    "count": { "type": "integer" }
                }
            }),
        );
        let provider = MockLlmProvider::new(vec![r#"{"count": 42}"#.to_string()]);

        let result = guardian
            .validate_with_retry(&skill, r#"{"count": "not a number"}"#, &provider)
            .await
            .unwrap();

        assert!(result.is_passed());
        assert_eq!(result.attempts, 2);
        assert_eq!(result.output.unwrap()["count"], 42);
    }

    // =====================
    // Log tests
    // =====================

    #[tokio::test]
    async fn test_logs_recorded_single_pass() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());

        let _ = guardian
            .validate_once(&skill, r#"{"content": "hello"}"#)
            .await;

        let logs = guardian.logs();
        assert_eq!(logs.len(), 1);
        assert!(logs[0].passed);
        assert!(logs[0].failed_layer.is_none());
    }

    #[tokio::test]
    async fn test_logs_recorded_with_retry() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());
        let provider = MockLlmProvider::new(vec![r#"{"content": "fixed"}"#.to_string()]);

        let _ = guardian
            .validate_with_retry(&skill, "bad json", &provider)
            .await;

        let logs = guardian.logs();
        assert_eq!(logs.len(), 2); // two attempts
        assert!(!logs[0].passed); // first attempt failed
        assert!(logs[0].failed_layer.is_some());
        assert!(logs[1].passed); // second attempt passed
        assert!(logs[1].failed_layer.is_none());
    }

    #[tokio::test]
    async fn test_logs_cleared() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());

        let _ = guardian
            .validate_once(&skill, r#"{"content": "hello"}"#)
            .await;

        assert!(!guardian.logs().is_empty());

        guardian.clear_logs();
        assert!(guardian.logs().is_empty());
    }

    #[tokio::test]
    async fn test_log_failed_layer_names() {
        let guardian = Guardian::with_defaults();
        let skill = make_skill("read_file", schema_with_content());

        // L1 failure
        let _ = guardian.validate_once(&skill, "not json").await;
        let logs = guardian.logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].failed_layer.as_deref(), Some("L1_format"));

        guardian.clear_logs();

        // L2 failure
        let _ = guardian.validate_once(&skill, r#"{"missing": true}"#).await;
        let logs = guardian.logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].failed_layer.as_deref(), Some("L2_schema"));
    }

    // =====================
    // Config customization tests
    // =====================

    #[tokio::test]
    async fn test_l4_disabled_passes_security() {
        let config = GuardianConfig {
            l4_security_check: false,
            ..GuardianConfig::default()
        };
        let guardian = Guardian::new(config);
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));

        let output = r#"{"content": "sk-abcdefghijklmnopqrstuvwxyz1234567890"}"#;
        let result = guardian.validate_once(&skill, output).await.unwrap();

        assert!(result.is_passed()); // L4 disabled → security not checked
    }

    #[tokio::test]
    async fn test_l2_disabled_skips_schema() {
        let config = GuardianConfig {
            l2_schema_check: false,
            ..GuardianConfig::default()
        };
        let guardian = Guardian::new(config);
        let skill = make_skill("read_file", schema_with_content());

        let result = guardian
            .validate_once(&skill, r#"{"missing": true}"#)
            .await
            .unwrap();

        assert!(result.is_passed()); // L2 disabled → schema not checked
    }

    #[tokio::test]
    async fn test_custom_max_retries() {
        let config = GuardianConfig {
            max_retries: 0, // No retries — only 1 attempt
            ..GuardianConfig::default()
        };
        let guardian = Guardian::new(config);
        let skill = make_skill("read_file", schema_with_content());
        let provider = MockLlmProvider::single("unused".to_string());

        let result = guardian
            .validate_with_retry(&skill, "bad json", &provider)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            GuardianError::MaxRetriesExceeded {
                attempts,
                max_retries,
                ..
            } => {
                assert_eq!(attempts, 1);
                assert_eq!(max_retries, 0);
            }
            other => panic!("expected MaxRetriesExceeded, got {other:?}"),
        }
        assert_eq!(provider.call_count(), 0); // No retries → LLM never called
    }

    #[tokio::test]
    async fn test_custom_output_size() {
        let config = GuardianConfig {
            max_output_size: 10,
            ..GuardianConfig::default()
        };
        let guardian = Guardian::new(config);
        let skill = make_skill("read_file", serde_json::json!({"type": "object"}));

        let long_output = r#"{"content": "this is a very long output that exceeds 10 bytes"}"#;
        let result = guardian.validate_once(&skill, long_output).await;

        // Truncation at 10 bytes breaks the JSON structure, so L1 format
        // check correctly fails. This is expected behavior — truncating
        // mid-JSON produces invalid JSON.
        assert!(result.is_err());

        // But the truncation warning should still be recorded in the log.
        let logs = guardian.logs();
        assert_eq!(logs.len(), 1);
        assert!(!logs[0].warnings.is_empty());
        assert!(logs[0].warnings[0].contains("truncated"));
    }

    // =====================
    // Debug tests
    // =====================

    #[test]
    fn test_guardian_debug() {
        let guardian = Guardian::with_defaults();
        let debug = format!("{guardian:?}");
        assert!(debug.contains("Guardian"));
        assert!(debug.contains("config"));
    }

    #[test]
    fn test_validation_log_entry_debug() {
        let entry = ValidationLogEntry {
            attempt: 1,
            failed_layer: Some("L1_format".to_string()),
            errors: vec!["not json".to_string()],
            warnings: vec![],
            passed: false,
            timestamp: 1234567890,
        };
        let debug = format!("{entry:?}");
        assert!(debug.contains("L1_format"));
        assert!(debug.contains("not json"));
    }
}
