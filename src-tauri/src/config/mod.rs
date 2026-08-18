//! Configuration management module — the single entry point for all config access.
//!
//! ## Usage
//!
//! ```no_run
//! use caspian_flow::config::ConfigManager;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let manager = ConfigManager::init().await?;
//! let settings = manager.settings();
//! println!("theme: {}", settings.app.theme);
//! # Ok(())
//! # }
//! ```

pub mod keystore;
pub mod migration;
pub mod paths;
pub mod settings;
pub mod validation;
pub mod watcher;

pub use paths::CaspianPaths;
pub use settings::{
    AppConfig, EmbeddingConfig, ModelConfig, PathsConfig, SecurityConfig, Settings,
    CURRENT_SCHEMA_VERSION,
};
pub use validation::{validate, ValidationResult};
pub use watcher::ConfigWatcher;

use std::path::Path;
use std::sync::Arc;

use crate::types::{AppError, AppResult};

/// The central configuration manager.
///
/// Owns the path resolution, file watcher, and provides thread-safe
/// access to the live settings.
pub struct ConfigManager {
    watcher: ConfigWatcher,
    paths: CaspianPaths,
}

impl ConfigManager {
    /// Initialize with default `~/.caspian/` paths.
    pub async fn init() -> AppResult<Self> {
        Self::init_with_paths(None).await
    }

    /// Initialize with a custom home directory.
    pub async fn init_with_paths(home: Option<&Path>) -> AppResult<Self> {
        let paths = CaspianPaths::resolve(home);
        paths.ensure_dirs()?;

        // Load or generate settings
        let settings = match Settings::load(&paths.settings_file) {
            Ok(s) => {
                // Validate
                match validation::validate(&s) {
                    Ok(result) if result.is_valid() => {
                        if !result.warnings.is_empty() {
                            tracing::info!(
                                warnings = ?result.warnings,
                                "config loaded with warnings"
                            );
                        }
                        s
                    }
                    Ok(result) => {
                        tracing::warn!(
                            errors = ?result.errors,
                            "config has validation errors, using defaults"
                        );
                        // Don't overwrite the file — let the user fix it
                        Settings::default_with_samples()
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "config validation error, using defaults");
                        Settings::default_with_samples()
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "config load failed, using defaults");
                Settings::default_with_samples()
            }
        };

        // Set up the watcher
        let watcher = ConfigWatcher::new(&paths.settings_file, settings)?;

        tracing::info!(
            path = %paths.settings_file.display(),
            "config manager initialized"
        );

        Ok(Self { watcher, paths })
    }

    /// Get the current settings (cheap Arc clone).
    pub fn settings(&self) -> Arc<Settings> {
        self.watcher.settings()
    }

    /// Get the paths.
    pub fn paths(&self) -> &CaspianPaths {
        &self.paths
    }

    /// Manually trigger a reload.
    pub fn reload(&self) -> AppResult<()> {
        self.watcher.manual_reload()
    }

    /// Register a callback for config reloads.
    pub fn on_reload(&self, callback: impl Fn(&Settings) + Send + Sync + 'static) {
        self.watcher.on_reload(callback);
    }

    /// Save settings to disk.
    pub fn save(&self, settings: &Settings) -> AppResult<()> {
        settings.save(&self.paths.settings_file)?;
        // The watcher will pick up the change automatically
        Ok(())
    }

    /// Update settings: save to disk and trigger reload.
    pub fn update(&self, settings: Settings) -> AppResult<()> {
        self.save(&settings)?;
        // Force immediate reload instead of waiting for watcher debounce
        self.reload()?;
        Ok(())
    }

    /// Resolve the API key for a model.
    pub fn resolve_api_key(&self, model_id: &str) -> AppResult<Option<String>> {
        let settings = self.settings();
        let model = settings.get_model(model_id).ok_or_else(|| {
            AppError::Config(crate::types::ConfigError::InvalidValue {
                field: "model_id".to_string(),
                reason: format!("model `{model_id}` not found in config"),
            })
        })?;

        keystore::resolve_api_key_string(model.api_key.as_deref(), model_id)
            .map_err(AppError::Config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_creates_default_config() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();

        let settings = manager.settings();
        assert_eq!(settings.schema_version, "1.0");
        assert!(settings.models.len() >= 3); // default_with_samples

        // File should exist on disk
        assert!(tmp.path().join("config/settings.yaml").exists());
    }

    #[tokio::test]
    async fn test_init_loads_existing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config/settings.yaml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let custom_yaml = r#"schema_version: "1.0"
app:
  theme: "light"
  language: "en-US"
"#;
        std::fs::write(&path, custom_yaml).unwrap();

        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        let settings = manager.settings();
        assert_eq!(settings.app.theme, "light");
        assert_eq!(settings.app.language, "en-US");
    }

    #[tokio::test]
    async fn test_init_falls_back_on_invalid_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();

        // Write invalid YAML (not parseable)
        std::fs::write(config_dir.join("settings.yaml"), "{{{invalid yaml").unwrap();

        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        let settings = manager.settings();
        // Should fall back to defaults
        assert_eq!(settings.schema_version, "1.0");
        assert_eq!(settings.app.theme, "dark");
    }

    #[tokio::test]
    async fn test_update_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();

        let mut new_settings = (*manager.settings()).clone();
        new_settings.app.theme = "light".to_string();

        manager.update(new_settings).unwrap();

        let reloaded = manager.settings();
        assert_eq!(reloaded.app.theme, "light");
    }

    #[tokio::test]
    async fn test_resolve_api_key_env_var() {
        std::env::set_var("TEST_CF_MANAGER_KEY", "manager-secret");
        let tmp = tempfile::tempdir().unwrap();

        let mut settings = Settings::default_with_samples();
        settings.models[0].api_key = Some("${TEST_CF_MANAGER_KEY}".to_string());
        settings
            .save(&tmp.path().join("config/settings.yaml"))
            .unwrap();
        // Need to create the dir first
        // Actually, let's use ConfigManager properly

        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        settings.save(&config_dir.join("settings.yaml")).unwrap();

        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        let key = manager.resolve_api_key("deepseek-chat").unwrap();
        assert_eq!(key, Some("manager-secret".to_string()));

        std::env::remove_var("TEST_CF_MANAGER_KEY");
    }

    #[tokio::test]
    async fn test_on_reload_callback() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();

        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter_clone = counter.clone();
        manager.on_reload(move |_| {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        // Trigger a reload
        manager.reload().unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_paths_created() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();

        let paths = manager.paths();
        assert!(paths.home.exists());
        assert!(paths.agents.exists());
        assert!(paths.skills.exists());
        assert!(paths.knowledge.exists());
        assert!(paths.logs.exists());
    }
}
