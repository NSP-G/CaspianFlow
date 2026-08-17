//! Settings struct and YAML read/write.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::paths::CaspianPaths;
use crate::types::{ConfigError, ConfigResult};

/// Current schema version.
pub const CURRENT_SCHEMA_VERSION: &str = "1.0";

/// Root configuration object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,

    #[serde(default)]
    pub app: AppConfig,

    #[serde(default)]
    pub models: Vec<ModelConfig>,

    #[serde(default)]
    pub paths: PathsConfig,

    #[serde(default)]
    pub embedding: EmbeddingConfig,

    #[serde(default)]
    pub security: SecurityConfig,
}

fn default_schema_version() -> String {
    CURRENT_SCHEMA_VERSION.to_string()
}

/// Application-level settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_theme")]
    pub theme: String,

    #[serde(default = "default_language")]
    pub language: String,

    #[serde(default = "default_agent")]
    pub default_agent: String,

    #[serde(default = "default_true")]
    pub auto_check_update: bool,
}

fn default_theme() -> String {
    "dark".to_string()
}
fn default_language() -> String {
    "zh-CN".to_string()
}
fn default_agent() -> String {
    "default".to_string()
}
fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            language: default_language(),
            default_agent: default_agent(),
            auto_check_update: default_true(),
        }
    }
}

/// Model configuration entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub provider: String,
    pub name: String,

    /// API key — can be `${VAR_NAME}` (env reference) or a direct string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Model name used in API calls (e.g. `qwen2.5:7b` for Ollama).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    #[serde(default = "default_priority")]
    pub priority: u32,

    /// Preset template name (e.g. "openai", "deepseek"). Used by P24 to
    /// auto-fill `base_url` / `auth_type` when omitted. `None` means a fully
    /// explicit (or custom) configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,

    /// Whether this model participates in P24 health probes. Defaults to true.
    #[serde(default = "default_true")]
    pub health: bool,

    /// Ordered fallback chain (model ids) tried when this model is unavailable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback: Vec<String>,

    /// Whether this is the global default model. Selected by
    /// `Settings::default_model()` when no caller-specified model is given.
    #[serde(default)]
    pub default: bool,

    /// Auth scheme override (e.g. "bearer", "x-api-key"). When `None`, derived
    /// from the preset. Used by P24 custom providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
}

fn default_max_tokens() -> u32 {
    4096
}
fn default_priority() -> u32 {
    1
}

/// Filesystem path configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    #[serde(default = "default_workspace")]
    pub workspace: String,

    #[serde(default = "default_caspian_home")]
    pub caspian_home: String,
}

fn default_workspace() -> String {
    "~/projects".to_string()
}
fn default_caspian_home() -> String {
    "~/.caspian".to_string()
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            caspian_home: default_caspian_home(),
        }
    }
}

/// Embedding model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_embedding_model")]
    pub model: String,

    #[serde(default = "default_batch_size")]
    pub max_batch_size: u32,

    /// Idle timeout in seconds before unloading the model from memory.
    /// 0 = never unload. Default: 600 (10 minutes).
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,

    /// Cache directory for embedding model files.
    /// If empty, defaults to `~/.caspian/models/`.
    #[serde(default)]
    pub cache_dir: String,

    /// Semantic routing: high confidence threshold for direct match.
    #[serde(default = "default_high_threshold")]
    pub high_threshold: f64,

    /// Semantic routing: low confidence threshold for candidate list.
    #[serde(default = "default_low_threshold")]
    pub low_threshold: f64,
}

fn default_embedding_model() -> String {
    "bge-small-zh-v1.5".to_string()
}
fn default_batch_size() -> u32 {
    32
}
fn default_idle_timeout() -> u64 {
    600
}
fn default_high_threshold() -> f64 {
    0.82
}
fn default_low_threshold() -> f64 {
    0.65
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: default_embedding_model(),
            max_batch_size: default_batch_size(),
            idle_timeout_secs: default_idle_timeout(),
            cache_dir: String::new(),
            high_threshold: default_high_threshold(),
            low_threshold: default_low_threshold(),
        }
    }
}

/// Security configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub require_confirmation_for_dangerous_skills: bool,

    #[serde(default = "default_shell_whitelist")]
    pub shell_whitelist: Vec<String>,

    #[serde(default)]
    pub network_allow_list: Vec<String>,
}

fn default_shell_whitelist() -> Vec<String> {
    vec![
        "ls".to_string(),
        "cat".to_string(),
        "grep".to_string(),
        "git".to_string(),
    ]
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            require_confirmation_for_dangerous_skills: default_true(),
            shell_whitelist: default_shell_whitelist(),
            network_allow_list: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Settings impl
// ---------------------------------------------------------------------------

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            app: AppConfig::default(),
            models: vec![],
            paths: PathsConfig::default(),
            embedding: EmbeddingConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

impl Settings {
    /// Generate the default configuration with sample models.
    pub fn default_with_samples() -> Self {
        let mut s = Self::default();
        s.models = vec![
            ModelConfig {
                id: "deepseek-chat".to_string(),
                provider: "deepseek".to_string(),
                name: "DeepSeek Chat".to_string(),
                api_key: Some("${DEEPSEEK_API_KEY}".to_string()),
                base_url: Some("https://api.deepseek.com/v1".to_string()),
                model_name: None,
                max_tokens: 4096,
                priority: 1,
                preset: Some("deepseek".to_string()),
                health: true,
                fallback: vec!["gpt-4o-mini".to_string()],
                default: true,
                auth_type: None,
            },
            ModelConfig {
                id: "gpt-4o-mini".to_string(),
                provider: "openai".to_string(),
                name: "GPT-4o Mini".to_string(),
                api_key: Some("${OPENAI_API_KEY}".to_string()),
                base_url: Some("https://api.openai.com/v1".to_string()),
                model_name: None,
                max_tokens: 4096,
                priority: 2,
                preset: Some("openai".to_string()),
                health: true,
                fallback: vec![],
                default: false,
                auth_type: None,
            },
            ModelConfig {
                id: "ollama-qwen2.5".to_string(),
                provider: "ollama".to_string(),
                name: "Qwen 2.5 7B".to_string(),
                api_key: None,
                base_url: Some("http://localhost:11434".to_string()),
                model_name: Some("qwen2.5:7b".to_string()),
                max_tokens: 4096,
                priority: 3,
                preset: Some("ollama".to_string()),
                health: false,
                fallback: vec![],
                default: false,
                auth_type: None,
            },
        ];
        s
    }

    /// Load settings from a YAML file.
    /// If the file doesn't exist, generates default config and writes it.
    pub fn load(path: &Path) -> ConfigResult<Self> {
        if !path.exists() {
            tracing::warn!(
                path = %path.display(),
                "settings file not found, generating default"
            );
            let settings = Self::default_with_samples();
            settings.save(path)?;
            return Ok(settings);
        }

        let contents =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Parse(e.to_string()))?;
        Self::from_yaml(&contents)
    }

    /// Parse settings from a YAML string.
    pub fn from_yaml(yaml: &str) -> ConfigResult<Self> {
        // If the file is empty, return defaults
        if yaml.trim().is_empty() {
            return Ok(Self::default());
        }

        // First try to parse as-is
        match serde_yaml::from_str::<Self>(yaml) {
            Ok(settings) => Ok(settings),
            Err(e) => {
                // Try to parse as a generic Value to detect schema_version
                let val: serde_yaml::Value =
                    serde_yaml::from_str(yaml).map_err(|e2| ConfigError::Parse(format!("{e2}")))?;

                let version = val
                    .get("schema_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                tracing::warn!(
                    version = %version,
                    error = %e,
                    "config parse failed, attempting migration"
                );

                // If migration is available, try it; otherwise error
                let migrated =
                    super::migration::try_migrate(yaml, &version, CURRENT_SCHEMA_VERSION)?;
                Ok(migrated)
            }
        }
    }

    /// Save settings to a YAML file.
    pub fn save(&self, path: &Path) -> ConfigResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Parse(e.to_string()))?;
        }
        let yaml = self.to_yaml()?;
        std::fs::write(path, yaml).map_err(|e| ConfigError::Parse(e.to_string()))?;
        Ok(())
    }

    /// Serialize settings to a YAML string.
    pub fn to_yaml(&self) -> ConfigResult<String> {
        serde_yaml::to_string(self).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Initialize settings for the given paths.
    /// Creates directories, loads or generates config.
    pub fn init(paths: &CaspianPaths) -> ConfigResult<Self> {
        paths
            .ensure_dirs()
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        Self::load(&paths.settings_file)
    }

    /// Find a model by id.
    pub fn get_model(&self, id: &str) -> Option<&ModelConfig> {
        self.models.iter().find(|m| m.id == id)
    }

    /// Get the highest-priority model (lowest priority number).
    pub fn primary_model(&self) -> Option<&ModelConfig> {
        self.models.iter().min_by_key(|m| m.priority)
    }

    /// Get the global default model (P24).
    ///
    /// Selects among models flagged `default: true`; when several are flagged,
    /// the highest-priority one (lowest `priority` number) wins. Returns `None`
    /// when no model is flagged as default.
    pub fn default_model(&self) -> Option<&ModelConfig> {
        self.models
            .iter()
            .filter(|m| m.default)
            .min_by_key(|m| m.priority)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"schema_version: "1.0"

app:
  theme: "dark"
  language: "zh-CN"
  default_agent: "default"
  auto_check_update: true

models:
  - id: "deepseek-chat"
    provider: "deepseek"
    name: "DeepSeek Chat"
    api_key: "${DEEPSEEK_API_KEY}"
    base_url: "https://api.deepseek.com/v1"
    max_tokens: 4096
    priority: 1

  - id: "ollama-qwen2.5"
    provider: "ollama"
    name: "Qwen 2.5 7B"
    model_name: "qwen2.5:7b"
    base_url: "http://localhost:11434"
    priority: 3

paths:
  workspace: "~/projects"
  caspian_home: "~/.caspian"

embedding:
  model: "bge-small-zh-v1.5"
  max_batch_size: 32

security:
  require_confirmation_for_dangerous_skills: true
  shell_whitelist: ["ls", "cat", "grep", "git"]
  network_allow_list: []
"#;

    #[test]
    fn test_load_from_yaml() {
        let settings = Settings::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(settings.schema_version, "1.0");
        assert_eq!(settings.app.theme, "dark");
        assert_eq!(settings.models.len(), 2);
        assert_eq!(settings.models[0].id, "deepseek-chat");
        assert_eq!(
            settings.models[0].api_key,
            Some("${DEEPSEEK_API_KEY}".to_string())
        );
        assert_eq!(settings.models[1].id, "ollama-qwen2.5");
        assert_eq!(settings.models[1].api_key, None);
        assert_eq!(settings.embedding.model, "bge-small-zh-v1.5");
        assert_eq!(settings.security.shell_whitelist.len(), 4);
    }

    #[test]
    fn test_roundtrip_yaml() {
        let settings = Settings::from_yaml(SAMPLE_YAML).unwrap();
        let yaml = settings.to_yaml().unwrap();
        let reparsed = Settings::from_yaml(&yaml).unwrap();
        assert_eq!(settings.schema_version, reparsed.schema_version);
        assert_eq!(settings.models.len(), reparsed.models.len());
        assert_eq!(settings.models[0].id, reparsed.models[0].id);
    }

    #[test]
    fn test_default_with_samples() {
        let settings = Settings::default_with_samples();
        assert_eq!(settings.schema_version, "1.0");
        assert_eq!(settings.models.len(), 3);
        assert_eq!(settings.app.theme, "dark");
    }

    #[test]
    fn test_empty_yaml_returns_default() {
        let settings = Settings::from_yaml("").unwrap();
        assert_eq!(settings.schema_version, "1.0");
        assert_eq!(settings.models.len(), 0);
    }

    #[test]
    fn test_load_creates_default_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.yaml");
        let settings = Settings::load(&path).unwrap();
        assert!(path.exists());
        assert_eq!(settings.models.len(), 3); // default_with_samples
    }

    #[test]
    fn test_save_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config/settings.yaml");
        let mut settings = Settings::default();
        settings.app.theme = "light".to_string();
        settings.save(&path).unwrap();

        let reloaded = Settings::load(&path).unwrap();
        assert_eq!(reloaded.app.theme, "light");
    }

    #[test]
    fn test_get_model() {
        let settings = Settings::default_with_samples();
        let model = settings.get_model("ollama-qwen2.5").unwrap();
        assert_eq!(model.provider, "ollama");
        assert!(settings.get_model("nonexistent").is_none());
    }

    #[test]
    fn test_primary_model() {
        let settings = Settings::default_with_samples();
        let primary = settings.primary_model().unwrap();
        assert_eq!(primary.id, "deepseek-chat");
        assert_eq!(primary.priority, 1);
    }

    #[test]
    fn test_default_model_selects_flagged() {
        let settings = Settings::default_with_samples();
        let def = settings.default_model().unwrap();
        // deepseek-chat is the only model flagged `default: true`.
        assert_eq!(def.id, "deepseek-chat");
        assert!(def.default);
    }

    #[test]
    fn test_default_model_none_when_unflagged() {
        let mut settings = Settings::default_with_samples();
        for m in settings.models.iter_mut() {
            m.default = false;
        }
        assert!(settings.default_model().is_none());
    }

    #[test]
    fn test_default_model_picks_highest_priority_among_flagged() {
        let mut settings = Settings::default_with_samples();
        // Flag both; gpt-4o-mini has priority 2, deepseek has priority 1,
        // so the highest-priority (lowest number) flagged one should win.
        settings.models[1].default = true;
        let def = settings.default_model().unwrap();
        assert_eq!(def.id, "deepseek-chat");
    }

    #[test]
    fn test_new_model_fields_default_via_partial_yaml() {
        // Backward-compatible: a YAML without the new fields still parses,
        // with sensible defaults.
        let yaml = r#"
schema_version: "1.0"
models:
  - id: legacy
    provider: openai
    name: Legacy
"#;
        let settings: Settings = serde_yaml::from_str(yaml).unwrap();
        let m = &settings.models[0];
        assert_eq!(m.preset, None);
        assert!(m.health); // default_true
        assert!(m.fallback.is_empty());
        assert!(!m.default);
        assert_eq!(m.auth_type, None);
    }

    #[test]
    fn test_partial_config_with_defaults() {
        let yaml = r#"
schema_version: "1.0"
app:
  theme: "light"
"#;
        let settings = Settings::from_yaml(yaml).unwrap();
        assert_eq!(settings.app.theme, "light");
        // Missing fields should use defaults
        assert_eq!(settings.app.language, "zh-CN");
        assert_eq!(settings.app.default_agent, "default");
        assert!(settings.app.auto_check_update);
    }
}
