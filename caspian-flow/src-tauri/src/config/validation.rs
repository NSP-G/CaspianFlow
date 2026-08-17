//! Configuration validation logic.

use std::collections::HashSet;

use super::settings::{Settings, CURRENT_SCHEMA_VERSION};
use crate::types::{ConfigError, ConfigResult};

/// Result of a validation pass: errors block startup, warnings are advisory.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn add_error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    pub fn add_warning(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

/// Validate a `Settings` instance.
///
/// - **Errors** (block startup): missing required fields, duplicate model IDs,
///   unsupported schema version.
/// - **Warnings** (advisory): out-of-range values that fall back to defaults,
///   insecure API key storage, etc.
pub fn validate(settings: &Settings) -> ConfigResult<ValidationResult> {
    let mut result = ValidationResult::default();

    // 1. Schema version check
    if settings.schema_version != CURRENT_SCHEMA_VERSION {
        result.add_warning(format!(
            "schema_version `{}` differs from current `{}` — migration recommended",
            settings.schema_version, CURRENT_SCHEMA_VERSION
        ));
    }

    // 2. App section
    let theme = settings.app.theme.as_str();
    if !["dark", "light", "system"].contains(&theme) {
        result.add_warning(format!(
            "app.theme `{}` is not a recognized value (dark/light/system), falling back to dark",
            theme
        ));
    }

    if settings.app.language.is_empty() {
        result.add_error("app.language must not be empty");
    }

    if settings.app.default_agent.is_empty() {
        result.add_error("app.default_agent must not be empty");
    }

    // 3. Models section
    let mut seen_ids = HashSet::new();
    for (i, model) in settings.models.iter().enumerate() {
        // Required fields
        if model.id.is_empty() {
            result.add_error(format!("models[{i}].id must not be empty"));
        }
        if model.provider.is_empty() {
            result.add_error(format!("models[{i}].provider must not be empty"));
        }
        if model.name.is_empty() {
            result.add_error(format!("models[{i}].name must not be empty"));
        }

        // Duplicate ID check
        if !seen_ids.insert(&model.id) {
            return Err(ConfigError::DuplicateModelId {
                id: model.id.clone(),
            });
        }

        // Provider-specific validation
        match model.provider.as_str() {
            "ollama" => {
                if model.base_url.is_none() {
                    result.add_warning(format!(
                        "models[{i}] (ollama) has no base_url, will default to http://localhost:11434"
                    ));
                }
            }
            "openai" | "deepseek" | "anthropic" | "azure" => {
                if model.api_key.is_none() {
                    result.add_warning(format!(
                        "models[{i}] ({}) has no api_key configured",
                        model.provider
                    ));
                }
                if model.base_url.is_none() {
                    result.add_warning(format!(
                        "models[{i}] ({}) has no base_url, will use provider default",
                        model.provider
                    ));
                }
            }
            _ => {
                // Unknown provider — not an error, just note it
                result.add_warning(format!(
                    "models[{i}].provider `{}` is not a built-in provider",
                    model.provider
                ));
            }
        }

        // Range checks
        if model.max_tokens == 0 {
            result.add_warning(format!(
                "models[{i}].max_tokens is 0, falling back to default 4096"
            ));
        }
        if model.max_tokens > 1_000_000 {
            result.add_warning(format!(
                "models[{i}].max_tokens `{}` is unusually large",
                model.max_tokens
            ));
        }

        // API key security check
        if let Some(ref key) = model.api_key {
            if !key.starts_with("${") || !key.ends_with('}') {
                result.add_warning(format!(
                    "models[{}].api_key appears to be a plaintext key — consider using ${{ENV_VAR}} reference",
                    i
                ));
            }
        }
    }

    // 4. Paths section
    if settings.paths.caspian_home.is_empty() {
        result.add_error("paths.caspian_home must not be empty");
    }

    // 5. Embedding section
    if settings.embedding.model.is_empty() {
        result.add_error("embedding.model must not be empty");
    }
    if settings.embedding.max_batch_size == 0 {
        result.add_warning("embedding.max_batch_size is 0, falling back to default 32");
    }
    if settings.embedding.max_batch_size > 256 {
        result.add_warning(format!(
            "embedding.max_batch_size `{}` is very large, may cause memory issues",
            settings.embedding.max_batch_size
        ));
    }

    // 6. Security section
    if settings.security.shell_whitelist.is_empty() {
        result.add_warning(
            "security.shell_whitelist is empty — all shell commands will require confirmation",
        );
    }

    if !result.is_valid() {
        // Return Ok with errors — the caller decides whether to block or fall back
        tracing::warn!(
            errors = ?result.errors,
            warnings = ?result.warnings,
            "config validation completed with errors"
        );
    } else if !result.warnings.is_empty() {
        tracing::info!(
            warnings = ?result.warnings,
            "config validation passed with warnings"
        );
    }

    Ok(result)
}

/// Validate and return the settings unchanged if valid, or return an error.
/// Warnings are logged but don't block.
pub fn validate_or_error(settings: &Settings) -> ConfigResult<()> {
    let result = validate(settings)?;
    if !result.is_valid() {
        return Err(ConfigError::InvalidValue {
            field: "settings".to_string(),
            reason: result.errors.join("; "),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::{ModelConfig, Settings};

    #[test]
    fn test_valid_default_config() {
        let settings = Settings::default_with_samples();
        let result = validate(&settings).unwrap();
        assert!(result.is_valid(), "errors: {:?}", result.errors);
    }

    #[test]
    fn test_missing_required_fields() {
        let mut settings = Settings::default();
        settings.app.language = String::new();
        settings.app.default_agent = String::new();
        settings.embedding.model = String::new();
        settings.paths.caspian_home = String::new();

        let result = validate(&settings).unwrap();
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.contains("app.language")));
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("app.default_agent")));
        assert!(result.errors.iter().any(|e| e.contains("embedding.model")));
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("paths.caspian_home")));
    }

    #[test]
    fn test_invalid_theme_warning() {
        let mut settings = Settings::default();
        settings.app.theme = "purple".to_string();
        let result = validate(&settings).unwrap();
        assert!(result.is_valid()); // warning, not error
        assert!(result.warnings.iter().any(|w| w.contains("theme")));
    }

    #[test]
    fn test_duplicate_model_id_error() {
        let mut settings = Settings::default();
        settings.models = vec![
            ModelConfig {
                id: "dup".to_string(),
                provider: "openai".to_string(),
                name: "First".to_string(),
                api_key: Some("${KEY}".to_string()),
                base_url: None,
                model_name: None,
                max_tokens: 4096,
                priority: 1,
                preset: None,
                health: true,
                fallback: vec![],
                default: false,
                auth_type: None,
            },
            ModelConfig {
                id: "dup".to_string(),
                provider: "openai".to_string(),
                name: "Second".to_string(),
                api_key: Some("${KEY}".to_string()),
                base_url: None,
                model_name: None,
                max_tokens: 4096,
                priority: 2,
                preset: None,
                health: true,
                fallback: vec![],
                default: false,
                auth_type: None,
            },
        ];

        let result = validate(&settings);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::DuplicateModelId { id } => assert_eq!(id, "dup"),
            other => panic!("expected DuplicateModelId, got {other:?}"),
        }
    }

    #[test]
    fn test_plaintext_api_key_warning() {
        let mut settings = Settings::default();
        settings.models = vec![ModelConfig {
            id: "test".to_string(),
            provider: "openai".to_string(),
            name: "Test".to_string(),
            api_key: Some("sk-1234567890abcdef".to_string()),
            base_url: None,
            model_name: None,
            max_tokens: 4096,
            priority: 1,
            preset: None,
            health: true,
            fallback: vec![],
            default: false,
            auth_type: None,
        }];

        let result = validate(&settings).unwrap();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("plaintext")));
    }

    #[test]
    fn test_env_var_api_key_no_warning() {
        let mut settings = Settings::default();
        settings.models = vec![ModelConfig {
            id: "test".to_string(),
            provider: "openai".to_string(),
            name: "Test".to_string(),
            api_key: Some("${OPENAI_API_KEY}".to_string()),
            base_url: None,
            model_name: None,
            max_tokens: 4096,
            priority: 1,
            preset: None,
            health: true,
            fallback: vec![],
            default: false,
            auth_type: None,
        }];

        let result = validate(&settings).unwrap();
        assert!(result.is_valid());
        assert!(!result.warnings.iter().any(|w| w.contains("plaintext")));
    }

    #[test]
    fn test_missing_api_key_for_cloud_provider() {
        let mut settings = Settings::default();
        settings.models = vec![ModelConfig {
            id: "test".to_string(),
            provider: "openai".to_string(),
            name: "Test".to_string(),
            api_key: None,
            base_url: None,
            model_name: None,
            max_tokens: 4096,
            priority: 1,
            preset: None,
            health: true,
            fallback: vec![],
            default: false,
            auth_type: None,
        }];

        let result = validate(&settings).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("no api_key")));
    }

    #[test]
    fn test_ollama_no_api_key_no_warning() {
        let mut settings = Settings::default();
        settings.models = vec![ModelConfig {
            id: "test".to_string(),
            provider: "ollama".to_string(),
            name: "Test".to_string(),
            api_key: None,
            base_url: Some("http://localhost:11434".to_string()),
            model_name: Some("qwen2.5:7b".to_string()),
            max_tokens: 4096,
            priority: 1,
            preset: None,
            health: true,
            fallback: vec![],
            default: false,
            auth_type: None,
        }];

        let result = validate(&settings).unwrap();
        assert!(!result.warnings.iter().any(|w| w.contains("no api_key")));
    }

    #[test]
    fn test_validate_or_error_passes() {
        let settings = Settings::default_with_samples();
        assert!(validate_or_error(&settings).is_ok());
    }

    #[test]
    fn test_validate_or_error_fails() {
        let mut settings = Settings::default();
        settings.app.language = String::new();
        assert!(validate_or_error(&settings).is_err());
    }

    #[test]
    fn test_max_batch_size_warnings() {
        let mut settings = Settings::default();
        settings.embedding.max_batch_size = 0;
        let result = validate(&settings).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("max_batch_size")));

        settings.embedding.max_batch_size = 500;
        let result = validate(&settings).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("very large")));
    }

    #[test]
    fn test_schema_version_mismatch_warning() {
        let mut settings = Settings::default();
        settings.schema_version = "0.9".to_string();
        let result = validate(&settings).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("schema_version")));
    }
}
