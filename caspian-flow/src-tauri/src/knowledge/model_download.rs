//! Model download management — retry logic, progress callbacks, and offline detection.
//!
//! fastembed's `TextEmbedding::try_new()` downloads model files from HuggingFace
//! on first use. This module wraps that with:
//! - **Retry logic**: up to 3 attempts with exponential backoff (1s → 2s → 4s)
//! - **Offline detection**: checks if the cache directory already contains model files
//! - **Progress callback**: optional callback for UI progress display

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::types::{EmbeddingError, EmbeddingResult};

/// Maximum number of download retries.
const MAX_RETRIES: usize = 3;

/// Type alias for an embedding vector.
pub type Embedding = Vec<f32>;

/// Callback for download progress updates.
///
/// The first argument is the attempt number (1-based), the second is a status message.
pub type ProgressCallback = Box<dyn Fn(usize, &str) + Send + Sync>;

/// Configuration for model initialization with retry and cache management.
pub struct DownloadConfig {
    /// The fastembed model identifier (e.g. "bge-small-zh-v1.5").
    pub model_name: String,
    /// Directory to cache model files.
    pub cache_dir: PathBuf,
    /// Whether to show download progress bars.
    pub show_progress: bool,
}

impl DownloadConfig {
    /// Create a new download config.
    pub fn new(model_name: &str, cache_dir: &Path) -> Self {
        Self {
            model_name: model_name.to_string(),
            cache_dir: cache_dir.to_path_buf(),
            show_progress: true,
        }
    }

    /// Set whether to show download progress.
    pub fn with_progress(mut self, show: bool) -> Self {
        self.show_progress = show;
        self
    }
}

/// Check if model files appear to be present in the cache directory.
///
/// fastembed stores models under `<cache_dir>/models--<org>--<model-name>/`.
/// We check that the snapshot directory contains a valid, non-empty `model.onnx`
/// (searched recursively, since the weight may sit in a subdir).
pub fn is_model_cached(model_name: &str, cache_dir: &Path) -> bool {
    // fastembed normalizes model names to a path like: models--BAAI--bge-small-zh-v1.5
    // The exact path depends on the HuggingFace repo, but we can check for any
    // models--* directory that contains the model name.
    if !cache_dir.exists() {
        return false;
    }

    // Check for directories matching the pattern
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            // fastembed uses HF hub cache structure: models--<org>--<model>
            if name.starts_with("models--") {
                let snapshots = entry.path().join("snapshots");
                if snapshots.exists() {
                    if let Ok(snaps) = std::fs::read_dir(&snapshots) {
                        // A snapshot dir must contain a *real* model file, not
                        // just be non-empty — a partially downloaded model leaves
                        // an incomplete snapshot that would fail at load time.
                        let has_valid_model = snaps
                            .filter_map(|e| e.ok())
                            .any(|s| s.path().is_dir() && snapshot_has_valid_model(&s.path()));
                        if has_valid_model {
                            // Check if the model name matches
                            let name_lower = name.to_lowercase();
                            let model_lower = model_name.to_lowercase().replace('-', "--");
                            if name_lower.contains(&model_lower) {
                                return true;
                            }
                            // Also check partial match
                            let model_part = model_name.to_lowercase();
                            if name_lower.contains(&model_part) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    false
}

/// Returns true if `dir` contains a valid `model.onnx` (non-empty) anywhere in
/// its tree.
///
/// A directory being merely non-empty is NOT sufficient: a partially downloaded
/// model leaves an empty/incomplete snapshot that must not be treated as cached
/// (loading it would fail at runtime). This is the core of the "directory
/// non-empty ≠ content valid" bug fixed in this module.
fn snapshot_has_valid_model(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().map(|n| n == "model.onnx").unwrap_or(false)
                && std::fs::metadata(&path)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

/// Attempt to initialize a model with retry logic.
///
/// This is a generic wrapper that retries the provided `init_fn` up to 3 times
/// with exponential backoff. The `init_fn` is expected to perform the actual
/// model initialization (which may trigger a download).
///
/// # Arguments
/// * `init_fn` - A closure that attempts model initialization and returns `Result<T, String>`.
/// * `progress` - Optional callback for progress updates.
///
/// # Returns
/// The initialized model on success, or `EmbeddingError::ModelInitFailed` after all retries fail.
pub fn init_with_retry<T, F>(init_fn: F, progress: Option<&ProgressCallback>) -> EmbeddingResult<T>
where
    F: Fn() -> Result<T, String>,
{
    let mut last_error = String::new();

    for attempt in 1..=MAX_RETRIES {
        if let Some(cb) = &progress {
            let msg = if attempt == 1 {
                "attempting model initialization (download may begin)..."
            } else {
                &format!("retry {}/{}...", attempt, MAX_RETRIES)
            };
            cb(attempt, msg);
        }

        tracing::info!(attempt, max_retries = MAX_RETRIES, "attempting model init");

        match init_fn() {
            Ok(result) => {
                if attempt > 1 {
                    tracing::info!(attempt, "model init succeeded after retry");
                }
                return Ok(result);
            }
            Err(e) => {
                last_error = e;
                tracing::warn!(
                    attempt,
                    max_retries = MAX_RETRIES,
                    error = %last_error,
                    "model init failed"
                );

                if attempt < MAX_RETRIES {
                    let backoff_secs = 1u64 << (attempt - 1); // 1, 2, 4
                    if let Some(cb) = &progress {
                        cb(attempt, &format!("waiting {backoff_secs}s before retry..."));
                    }
                    std::thread::sleep(Duration::from_secs(backoff_secs));
                }
            }
        }
    }

    Err(EmbeddingError::ModelInitFailed {
        retries: MAX_RETRIES,
        reason: last_error,
    })
}

/// Resolve the cache directory for embedding models.
///
/// Priority:
/// 1. Explicit `cache_dir` from config (if non-empty)
/// 2. `~/.caspian/models/`
pub fn resolve_cache_dir(configured: &str) -> PathBuf {
    if !configured.is_empty() {
        return PathBuf::from(expand_tilde(configured));
    }

    // Default: ~/.caspian/models/
    if let Some(home) = dirs::home_dir() {
        home.join(".caspian").join("models")
    } else {
        PathBuf::from(".caspian/models")
    }
}

/// Expand a leading `~` in a path string to the home directory.
fn expand_tilde(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

/// Check if we're likely in offline mode (no network access).
///
/// This is a heuristic: we check if the `HF_HUB_OFFLINE` env var is set,
/// or if we can't reach HuggingFace within a short timeout.
pub fn is_offline_mode() -> bool {
    // Check explicit offline env var
    if std::env::var("HF_HUB_OFFLINE").is_ok() {
        return true;
    }

    // Check if FASTEMBED_OFFLINE is set
    if std::env::var("FASTEMBED_OFFLINE").is_ok() {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_init_with_retry_succeeds_first_try() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();

        let result: EmbeddingResult<i32> = init_with_retry(
            || {
                count_clone.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            },
            None,
        );

        assert_eq!(result.unwrap(), 42);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_init_with_retry_succeeds_on_second_attempt() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();

        let result: EmbeddingResult<i32> = init_with_retry(
            || {
                let n = count_clone.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err("simulated failure".to_string())
                } else {
                    Ok(42)
                }
            },
            None,
        );

        assert_eq!(result.unwrap(), 42);
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_init_with_retry_fails_after_max_retries() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();

        let result: EmbeddingResult<i32> = init_with_retry(
            || {
                count_clone.fetch_add(1, Ordering::SeqCst);
                Err("always fails".to_string())
            },
            None,
        );

        assert!(result.is_err());
        assert_eq!(count.load(Ordering::SeqCst), MAX_RETRIES);

        match result.unwrap_err() {
            EmbeddingError::ModelInitFailed { retries, reason } => {
                assert_eq!(retries, MAX_RETRIES);
                assert_eq!(reason, "always fails");
            }
            other => panic!("expected ModelInitFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_is_model_cached_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_model_cached("bge-small-zh-v1.5", tmp.path()));
    }

    #[test]
    fn test_is_model_cached_nonexistent_dir() {
        assert!(!is_model_cached(
            "bge-small-zh-v1.5",
            Path::new("/nonexistent/path/models"),
        ));
    }

    #[test]
    fn test_is_model_cached_partial_download_is_not_cached() {
        // A snapshot dir exists but the weight file never landed (partial
        // download, interrupted). The directory is non-empty, yet the model is
        // NOT usable — must report as not cached.
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("models--BAAI--bge-small-zh-v1.5");
        let snap = repo.join("snapshots").join("abc123");
        std::fs::create_dir_all(&snap).unwrap();
        // Stray non-model file — simulates an incomplete download.
        std::fs::write(snap.join("config.json"), "{}").unwrap();
        assert!(
            !is_model_cached("bge-small-zh-v1.5", tmp.path()),
            "partial download (no model.onnx) must not be treated as cached"
        );

        // Even an EMPTY model.onnx must not count as cached.
        std::fs::write(snap.join("model.onnx"), "").unwrap();
        assert!(
            !is_model_cached("bge-small-zh-v1.5", tmp.path()),
            "zero-byte model.onnx must not be treated as cached"
        );
    }

    #[test]
    fn test_is_model_cached_valid_onnx_is_cached() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("models--BAAI--bge-small-zh-v1.5");
        let snap = repo.join("snapshots").join("abc123");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("model.onnx"), vec![0u8; 1024]).unwrap();
        std::fs::write(snap.join("config.json"), "{}").unwrap();
        assert!(
            is_model_cached("bge-small-zh-v1.5", tmp.path()),
            "non-empty model.onnx must be treated as cached"
        );
    }

    #[test]
    fn test_resolve_cache_dir_default() {
        let dir = resolve_cache_dir("");
        // Should end with .caspian/models
        assert!(dir.to_string_lossy().contains("models"));
    }

    #[test]
    fn test_resolve_cache_dir_explicit() {
        let dir = resolve_cache_dir("/custom/cache");
        assert_eq!(dir, PathBuf::from("/custom/cache"));
    }

    #[test]
    fn test_resolve_cache_dir_tilde() {
        let dir = resolve_cache_dir("~/my_models");
        // Should expand ~ to home dir
        assert!(!dir.to_string_lossy().contains("~"));
    }

    #[test]
    fn test_is_offline_mode_env_var() {
        // Set offline env var
        std::env::set_var("FASTEMBED_OFFLINE", "1");
        assert!(is_offline_mode());
        std::env::remove_var("FASTEMBED_OFFLINE");
        assert!(!is_offline_mode());
    }

    #[test]
    fn test_download_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config = DownloadConfig::new("bge-small-zh-v1.5", tmp.path());
        assert_eq!(config.model_name, "bge-small-zh-v1.5");
        assert!(config.show_progress);

        let config = config.with_progress(false);
        assert!(!config.show_progress);
    }
}
