//! Configuration IPC commands.
//!
//! These functions are designed to be annotated with `#[tauri::command]`
//! once the Tauri runtime is integrated. For now they are plain async
//! functions that can be called from Rust or wrapped by the frontend bridge.

use std::sync::Arc;

use crate::config::{ConfigManager, Settings, ValidationResult};
use crate::types::AppResult;

/// Get the current settings as a serializable struct.
pub async fn get_settings(manager: &ConfigManager) -> AppResult<Arc<Settings>> {
    Ok(manager.settings())
}

/// Update settings: saves to disk and triggers hot reload.
pub async fn update_settings(manager: &ConfigManager, settings: Settings) -> AppResult<()> {
    manager.update(settings)
}

/// Reload settings from disk (picks up external file changes).
pub async fn reload_settings(manager: &ConfigManager) -> AppResult<()> {
    manager.reload()
}

/// Validate the current settings without saving.
pub async fn validate_settings(manager: &ConfigManager) -> AppResult<ValidationResult> {
    let settings = manager.settings();
    Ok(crate::config::validate(&settings)?)
}

/// Resolve the API key for a given model (returns whether it's available,
/// but not the actual key value — for security).
pub async fn check_api_key(manager: &ConfigManager, model_id: &str) -> AppResult<bool> {
    match manager.resolve_api_key(model_id) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(_) => Ok(false),
    }
}

/// Store an API key in the OS keychain for a given model.
pub async fn store_api_key(model_id: &str, api_key: &str) -> AppResult<()> {
    crate::config::keystore::store_in_keychain(model_id, api_key)
        .map_err(crate::types::AppError::Config)
}

/// Delete an API key from the OS keychain.
pub async fn delete_api_key(model_id: &str) -> AppResult<()> {
    crate::config::keystore::delete_from_keychain(model_id).map_err(crate::types::AppError::Config)
}

/// Get the current schema version.
pub fn get_schema_version() -> &'static str {
    crate::config::CURRENT_SCHEMA_VERSION
}

/// Get the settings file path as a string.
pub fn get_settings_path(manager: &ConfigManager) -> String {
    manager.paths().settings_display()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        let settings = get_settings(&manager).await.unwrap();
        assert_eq!(settings.schema_version, "1.0");
    }

    #[tokio::test]
    async fn test_update_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();

        let mut new_settings = (*manager.settings()).clone();
        new_settings.app.theme = "light".to_string();

        update_settings(&manager, new_settings).await.unwrap();

        let reloaded = get_settings(&manager).await.unwrap();
        assert_eq!(reloaded.app.theme, "light");
    }

    #[tokio::test]
    async fn test_validate_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        let result = validate_settings(&manager).await.unwrap();
        assert!(result.is_valid());
    }

    #[tokio::test]
    async fn test_check_api_key_no_key() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        // Ollama model has no API key
        let has_key = check_api_key(&manager, "ollama-qwen2.5").await.unwrap();
        assert!(!has_key);
    }

    #[tokio::test]
    async fn test_check_api_key_with_env() {
        std::env::set_var("TEST_CF_CMD_KEY", "test-cmd-key");
        let tmp = tempfile::tempdir().unwrap();

        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let mut settings = Settings::default_with_samples();
        settings.models[0].api_key = Some("${TEST_CF_CMD_KEY}".to_string());
        settings.save(&config_dir.join("settings.yaml")).unwrap();

        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        let has_key = check_api_key(&manager, "deepseek-chat").await.unwrap();
        assert!(has_key);

        std::env::remove_var("TEST_CF_CMD_KEY");
    }

    #[test]
    fn test_get_schema_version() {
        assert_eq!(get_schema_version(), "1.0");
    }

    #[tokio::test]
    async fn test_get_settings_path() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::init_with_paths(Some(tmp.path()))
            .await
            .unwrap();
        let path = get_settings_path(&manager);
        assert!(path.ends_with("settings.yaml"));
    }
}
