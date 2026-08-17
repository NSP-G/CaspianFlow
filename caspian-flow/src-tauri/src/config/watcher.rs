//! Configuration file watcher with debounce for hot reload.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify_debouncer_mini::new_debouncer;
use parking_lot::RwLock;

use super::settings::Settings;
use super::validation;
use crate::types::{AppError, AppResult};

/// Callback invoked when the config is reloaded.
pub type ReloadCallback = Arc<dyn Fn(&Settings) + Send + Sync>;

/// Watches `settings.yaml` for changes and reloads with debounce.
pub struct ConfigWatcher {
    /// The current live settings (atomically swapped on reload).
    settings: Arc<ArcSwap<Settings>>,
    /// The file path being watched.
    path: PathBuf,
    /// The debouncer — type-erased, kept alive to keep watching.
    ///
    /// Wrapped in a `Mutex` purely to restore `Sync`: the underlying `notify`
    /// watcher is `!Sync`, which would otherwise make `ConfigManager` `!Sync`
    /// and break consumers that need it inside `Send` async methods (e.g.
    /// `ModelRouter` held by `SqliteKnowledgeQA`). We never lock it — it is
    /// purely RAII; the `Mutex` only provides the `Sync` auto-trait.
    _debouncer: std::sync::Mutex<Option<Box<dyn Any + Send>>>,
    /// Callbacks invoked after a successful reload.
    callbacks: Arc<RwLock<Vec<ReloadCallback>>>,
}

impl ConfigWatcher {
    /// Create a new watcher for the given path with the given initial settings.
    pub fn new(path: &Path, initial: Settings) -> AppResult<Self> {
        let settings = Arc::new(ArcSwap::from_pointee(initial));
        let callbacks = Arc::new(RwLock::new(Vec::new()));

        let watcher_settings = settings.clone();
        let watcher_path = path.to_path_buf();
        let watcher_callbacks = callbacks.clone();

        let debouncer: Option<Box<dyn Any + Send>> = if path.exists() {
            let (tx, rx) = std::sync::mpsc::channel();

            let mut deb =
                new_debouncer(Duration::from_millis(500), tx).map_err(AppError::Notify)?;

            // Watch the parent directory (watching the file itself can miss edits on some platforms)
            if let Some(parent) = path.parent() {
                deb.watcher()
                    .watch(parent, notify::RecursiveMode::NonRecursive)
                    .map_err(AppError::Notify)?;
            }

            // Spawn the event handler thread
            std::thread::spawn(move || {
                while let Ok(events_result) = rx.recv() {
                    match events_result {
                        Ok(events) => {
                            for event in events {
                                if event.path == watcher_path {
                                    tracing::debug!("config file change detected, reloading");
                                    Self::do_reload(
                                        &watcher_path,
                                        &watcher_settings,
                                        &watcher_callbacks,
                                    );
                                }
                            }
                        }
                        Err(errors) => {
                            tracing::warn!(
                                errors = ?errors,
                                "file watcher errors"
                            );
                        }
                    }
                }
            });

            Some(Box::new(deb))
        } else {
            tracing::warn!(path = %path.display(), "config file does not exist, watcher disabled");
            None
        };

        Ok(Self {
            settings,
            path: path.to_path_buf(),
            _debouncer: std::sync::Mutex::new(debouncer),
            callbacks,
        })
    }

    /// Internal: reload settings from disk, validate, and swap if valid.
    fn do_reload(
        path: &Path,
        settings: &Arc<ArcSwap<Settings>>,
        callbacks: &Arc<RwLock<Vec<ReloadCallback>>>,
    ) {
        match Settings::load(path) {
            Ok(new_settings) => match validation::validate(&new_settings) {
                Ok(result) if result.is_valid() => {
                    settings.store(Arc::new(new_settings.clone()));
                    tracing::info!("config hot-reloaded successfully");
                    let cbs = callbacks.read();
                    for cb in cbs.iter() {
                        cb(&new_settings);
                    }
                }
                Ok(result) => {
                    tracing::warn!(
                        errors = ?result.errors,
                        "config reload rejected: validation failed, keeping old config"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "config reload rejected: validation error, keeping old config"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "config reload failed, keeping old config"
                );
            }
        }
    }

    /// Get the current settings (cheap Arc clone).
    pub fn settings(&self) -> Arc<Settings> {
        self.settings.load_full()
    }

    /// Register a callback to be invoked after a successful reload.
    pub fn on_reload(&self, callback: impl Fn(&Settings) + Send + Sync + 'static) {
        self.callbacks.write().push(Arc::new(callback));
    }

    /// Manually trigger a reload (useful for testing).
    pub fn manual_reload(&self) -> AppResult<()> {
        let new_settings = Settings::load(&self.path)?;
        match validation::validate(&new_settings) {
            Ok(result) if result.is_valid() => {
                self.settings.store(Arc::new(new_settings.clone()));
                tracing::info!("config manually reloaded");
                let cbs = self.callbacks.read();
                for cb in cbs.iter() {
                    cb(&new_settings);
                }
                Ok(())
            }
            Ok(result) => Err(AppError::Config(crate::types::ConfigError::InvalidValue {
                field: "settings".to_string(),
                reason: result.errors.join("; "),
            })),
            Err(e) => Err(AppError::Config(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watcher_initial_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.yaml");
        let settings = Settings::default_with_samples();
        settings.save(&path).unwrap();

        let watcher = ConfigWatcher::new(&path, settings.clone()).unwrap();
        let live = watcher.settings();
        assert_eq!(live.schema_version, "1.0");
        assert_eq!(live.models.len(), 3);
    }

    #[test]
    fn test_watcher_manual_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.yaml");
        let settings = Settings::default_with_samples();
        settings.save(&path).unwrap();

        let watcher = ConfigWatcher::new(&path, settings).unwrap();

        // Modify the config file
        let mut modified = Settings::default_with_samples();
        modified.app.theme = "light".to_string();
        modified.save(&path).unwrap();

        // Manual reload
        watcher.manual_reload().unwrap();

        let live = watcher.settings();
        assert_eq!(live.app.theme, "light");
    }

    #[test]
    fn test_watcher_callback() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.yaml");
        let settings = Settings::default_with_samples();
        settings.save(&path).unwrap();

        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter_clone = counter.clone();

        let watcher = ConfigWatcher::new(&path, settings).unwrap();
        watcher.on_reload(move |_s| {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        watcher.manual_reload().unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn test_watcher_rejects_invalid_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.yaml");
        let settings = Settings::default_with_samples();
        settings.save(&path).unwrap();

        let watcher = ConfigWatcher::new(&path, settings.clone()).unwrap();

        // Write an invalid config (empty language)
        let invalid_yaml = r#"schema_version: "1.0"
app:
  language: ""
  default_agent: ""
"#;
        std::fs::write(&path, invalid_yaml).unwrap();

        // Reload should fail
        let result = watcher.manual_reload();
        assert!(result.is_err());

        // Old config should be preserved
        let live = watcher.settings();
        assert_eq!(live.app.language, "zh-CN");
        assert_eq!(live.models.len(), 3);
    }

    #[test]
    fn test_watcher_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nonexistent/settings.yaml");
        let settings = Settings::default();

        // Should not panic, watcher just disabled
        let watcher = ConfigWatcher::new(&path, settings).unwrap();
        assert_eq!(watcher.settings().schema_version, "1.0");
    }
}
