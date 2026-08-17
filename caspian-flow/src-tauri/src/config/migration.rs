//! Configuration version migration framework.

use super::settings::Settings;
use crate::types::{ConfigError, ConfigResult};

/// A single migration step from one schema version to the next.
pub trait Migration: Send + Sync {
    /// The version this migration upgrades **to**.
    fn target_version(&self) -> &str;

    /// Apply the migration to a raw YAML string, returning the updated YAML.
    fn migrate(&self, yaml: &str) -> ConfigResult<String>;
}

/// Pipeline that chains multiple migrations in sequence.
pub struct MigrationPipeline {
    migrations: Vec<Box<dyn Migration>>,
}

impl MigrationPipeline {
    pub fn new() -> Self {
        Self {
            migrations: Vec::new(),
        }
    }

    pub fn register(mut self, migration: Box<dyn Migration>) -> Self {
        self.migrations.push(migration);
        self
    }

    /// Run all applicable migrations from `from_version` to `to_version`.
    pub fn run(&self, yaml: &str, from_version: &str, to_version: &str) -> ConfigResult<String> {
        let mut current_yaml = yaml.to_string();
        let mut current_version = from_version.to_string();

        // Sort migrations by target version (they should already be registered in order,
        // but we sort to be safe)
        let applicable: Vec<_> = self
            .migrations
            .iter()
            .filter(|m| {
                version_cmp(m.target_version(), &current_version) > 0
                    && version_cmp(m.target_version(), to_version) <= 0
            })
            .collect();

        for migration in &applicable {
            tracing::info!(
                from = %current_version,
                to = %migration.target_version(),
                "running config migration"
            );
            current_yaml =
                migration
                    .migrate(&current_yaml)
                    .map_err(|e| ConfigError::Migration {
                        version: migration.target_version().to_string(),
                        reason: e.to_string(),
                    })?;
            current_version = migration.target_version().to_string();
        }

        Ok(current_yaml)
    }
}

impl Default for MigrationPipeline {
    fn default() -> Self {
        Self::new().register(Box::new(MigrateV0_9ToV1_0))
    }
}

/// Compare two semver-like version strings.
/// Returns >0 if a > b, 0 if equal, <0 if a < b.
fn version_cmp(a: &str, b: &str) -> i32 {
    let parse = |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse().ok()).collect() };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..va.len().max(vb.len()) {
        let na = va.get(i).copied().unwrap_or(0);
        let nb = vb.get(i).copied().unwrap_or(0);
        if na != nb {
            return na as i32 - nb as i32;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Migration: v0.9 → v1.0
// ---------------------------------------------------------------------------

/// Migration from schema v0.9 to v1.0.
///
/// Changes in v1.0:
/// - `schema_version` field renamed from `config_version` to `schema_version`
/// - `app.auto_update` renamed to `app.auto_check_update`
/// - `models[].api_base` renamed to `models[].base_url`
/// - Default `embedding` and `security` sections added
pub struct MigrateV0_9ToV1_0;

impl Migration for MigrateV0_9ToV1_0 {
    fn target_version(&self) -> &str {
        "1.0"
    }

    fn migrate(&self, yaml: &str) -> ConfigResult<String> {
        let mut val: serde_yaml::Value =
            serde_yaml::from_str(yaml).map_err(|e| ConfigError::Parse(e.to_string()))?;

        let map = val
            .as_mapping_mut()
            .ok_or_else(|| ConfigError::Parse("config root is not a mapping".to_string()))?;

        // 1. Rename `config_version` → `schema_version`
        if let Some(v) = map.remove(serde_yaml::Value::String("config_version".to_string())) {
            map.insert(serde_yaml::Value::String("schema_version".to_string()), v);
        }

        // Ensure schema_version is set to 1.0
        map.insert(
            serde_yaml::Value::String("schema_version".to_string()),
            serde_yaml::Value::String("1.0".to_string()),
        );

        // 2. Rename app.auto_update → app.auto_check_update
        if let Some(app_val) = map.get_mut(serde_yaml::Value::String("app".to_string())) {
            if let Some(app_map) = app_val.as_mapping_mut() {
                if let Some(v) =
                    app_map.remove(serde_yaml::Value::String("auto_update".to_string()))
                {
                    app_map.insert(
                        serde_yaml::Value::String("auto_check_update".to_string()),
                        v,
                    );
                }
            }
        }

        // 3. Rename models[].api_base → models[].base_url
        if let Some(models_val) = map.get_mut(serde_yaml::Value::String("models".to_string())) {
            if let Some(models_seq) = models_val.as_sequence_mut() {
                for model in models_seq {
                    if let Some(model_map) = model.as_mapping_mut() {
                        if let Some(v) =
                            model_map.remove(serde_yaml::Value::String("api_base".to_string()))
                        {
                            model_map.insert(serde_yaml::Value::String("base_url".to_string()), v);
                        }
                    }
                }
            }
        }

        // 4. Serialize back and parse through Settings to fill defaults
        let migrated_yaml =
            serde_yaml::to_string(&val).map_err(|e| ConfigError::Parse(e.to_string()))?;

        // Parse through Settings to get default-filled struct, then re-serialize
        let settings: Settings =
            serde_yaml::from_str(&migrated_yaml).map_err(|e| ConfigError::Parse(e.to_string()))?;

        settings.to_yaml()
    }
}

/// Try to migrate a YAML config from `from_version` to `to_version`.
/// Uses the default migration pipeline.
pub fn try_migrate(yaml: &str, from_version: &str, to_version: &str) -> ConfigResult<Settings> {
    if from_version == to_version {
        // Same version — shouldn't happen (parse should have succeeded)
        return Settings::from_yaml(yaml);
    }

    if version_cmp(from_version, to_version) > 0 {
        // Downgrade — not supported
        return Err(ConfigError::Migration {
            version: to_version.to_string(),
            reason: format!(
                "downgrade from {from_version} to {to_version} is not supported — \
                 please manually update your config or delete it to regenerate defaults"
            ),
        });
    }

    let pipeline = MigrationPipeline::default();
    let migrated_yaml = pipeline.run(yaml, from_version, to_version)?;

    tracing::info!(
        from = from_version,
        to = to_version,
        "config migration completed"
    );

    Settings::from_yaml(&migrated_yaml)
}

/// Get a list of registered migration versions.
pub fn registered_migrations() -> Vec<&'static str> {
    vec!["1.0"]
}

#[cfg(test)]
mod tests {
    use super::*;

    const V0_9_YAML: &str = r#"config_version: "0.9"

app:
  theme: "dark"
  language: "en-US"
  default_agent: "default"
  auto_update: true

models:
  - id: "gpt-4"
    provider: "openai"
    name: "GPT-4"
    api_key: "${OPENAI_API_KEY}"
    api_base: "https://api.openai.com/v1"
    max_tokens: 8192
    priority: 1
"#;

    #[test]
    fn test_version_cmp() {
        assert!(version_cmp("1.0", "0.9") > 0);
        assert!(version_cmp("0.9", "1.0") < 0);
        assert_eq!(version_cmp("1.0", "1.0"), 0);
        assert!(version_cmp("1.1", "1.0") > 0);
        assert!(version_cmp("2.0", "1.99") > 0);
    }

    #[test]
    fn test_migrate_v0_9_to_v1_0() {
        let settings = try_migrate(V0_9_YAML, "0.9", "1.0").unwrap();

        // schema_version should be updated
        assert_eq!(settings.schema_version, "1.0");

        // auto_update → auto_check_update
        assert!(settings.app.auto_check_update);

        // api_base → base_url
        assert_eq!(
            settings.models[0].base_url,
            Some("https://api.openai.com/v1".to_string())
        );

        // Preserved fields
        assert_eq!(settings.app.theme, "dark");
        assert_eq!(settings.app.language, "en-US");
        assert_eq!(settings.models[0].id, "gpt-4");
        assert_eq!(
            settings.models[0].api_key,
            Some("${OPENAI_API_KEY}".to_string())
        );
    }

    #[test]
    fn test_migrate_adds_missing_sections() {
        // V0.9 config without embedding/security sections
        let yaml = r#"config_version: "0.9"
app:
  theme: "light"
"#;
        let settings = try_migrate(yaml, "0.9", "1.0").unwrap();

        // Should have default embedding and security sections
        assert_eq!(settings.embedding.model, "bge-small-zh-v1.5");
        assert!(settings.security.require_confirmation_for_dangerous_skills);
    }

    #[test]
    fn test_migrate_downgrade_not_supported() {
        let result = try_migrate(V0_9_YAML, "1.0", "0.9");
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::Migration { reason, .. } => {
                assert!(reason.contains("downgrade"));
            }
            other => panic!("expected Migration error, got {other:?}"),
        }
    }

    #[test]
    fn test_migrate_same_version() {
        let yaml = r#"schema_version: "1.0"
app:
  theme: "dark"
"#;
        let settings = try_migrate(yaml, "1.0", "1.0").unwrap();
        assert_eq!(settings.schema_version, "1.0");
        assert_eq!(settings.app.theme, "dark");
    }

    #[test]
    fn test_pipeline_empty() {
        let pipeline = MigrationPipeline::new();
        let result = pipeline.run("test", "1.0", "1.0").unwrap();
        assert_eq!(result, "test");
    }

    #[test]
    fn test_pipeline_skips_inapplicable() {
        // If from_version is already >= target, no migration runs
        let pipeline = MigrationPipeline::default();
        let yaml = r#"schema_version: "1.0"
app:
  theme: "dark"
"#;
        let result = pipeline.run(yaml, "1.0", "1.0").unwrap();
        // Should be unchanged (no migrations applied)
        assert!(result.contains("schema_version"));
    }
}
