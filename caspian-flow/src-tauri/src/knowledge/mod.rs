//! Knowledge module — local embedding model integration (P11) **and** the P22
//! keyword knowledge base (RAG with FTS5).
//!
//! ## Sub-modules
//!
//! - `embedder` / `model_download` / `similarity` — P11 semantic-routing
//!   embedding stack. **P22 does not touch these** (decided D6/R1): the
//!   knowledge base is keyword-only and depends on `LlmProvider`, not on
//!   vector embeddings.
//! - `chunker` — pure text chunking (800/80, paragraph-boundary fallback) and
//!   CJK bigram preprocessing for FTS5.
//! - `schema` — SQLite DDL + `user_version` migration for the knowledge DB.
//! - `store` — `KnowledgeStore` trait + `SqliteKnowledgeStore` (import /
//!   list / delete / keyword search, contentless FTS5).
//! - `qa` — `KnowledgeQAService` trait + `SqliteKnowledgeQA` (retrieve +
//!   `LlmProvider` post-processing → cited answer).
//!
//! ## P22 design constraints (locked before implementation)
//!
//! - **No new dependencies** (D1): SQLite+FTS5 come from P21's `rusqlite`;
//!   `sha2` from P20; `uuid`/`chrono` from P21.
//! - **Chinese works** (D8): `unicode61` indexes whole CJK runs as one token,
//!   so content is bigram-preprocessed before indexing/querying.
//! - **Storage kept lean** (R2): no redundant `bigram` column; contentless
//!   FTS5 (`content=''`), ~67% smaller than the design-doc's original schema.

pub mod chunker;
pub mod embedder;
pub mod error;
pub mod model_download;
pub mod qa;
pub mod schema;
pub mod similarity;
pub mod store;

pub use chunker::{
    bigram, chunk_text, fts_query, Chunk, DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE,
};
pub use error::{KnowledgeError, KnowledgeResult};
pub use qa::{
    KnowledgeQAService, QAMeta, QAResponse, RetrievalStrategy, Source, SqliteKnowledgeQA,
    MAX_SNIPPET_LEN,
};
pub use schema::{init_db, CURRENT_SCHEMA};
pub use store::{
    Document, DocumentSearchResult, KnowledgeStore, SearchResult, SemanticSearchResult,
    SqliteKnowledgeStore,
};

pub use embedder::Embedder;
pub use model_download::{init_with_retry, is_model_cached, is_offline_mode, resolve_cache_dir};
pub use similarity::{
    cosine_similarity, euclidean_distance, l2_normalize, normalized, top_k_by_similarity,
};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::config::settings::EmbeddingConfig;
use crate::types::{AppResult, EmbeddingError, EmbeddingResult};

/// The central embedding service — manages model lifecycle.
///
/// Features:
/// - **Lazy loading**: model is loaded on first `embed()` call
/// - **Preheating**: `preheat()` triggers background loading without blocking
/// - **Idle unloading**: after `idle_timeout_secs` of inactivity, the model is dropped
/// - **Model switching**: `set_model()` unloads the current model; next call lazy-loads the new one
/// - **Thread-safe**: `RwLock<Option<Embedder>>` allows concurrent reads after load
pub struct EmbeddingService {
    /// The loaded embedder, if any. None = not loaded or unloaded.
    inner: Arc<RwLock<Option<Embedder>>>,
    /// Configuration (model name, cache dir, batch size, timeouts).
    config: Arc<RwLock<EmbeddingConfig>>,
    /// Timestamp of the last embed/access call.
    last_access: Arc<RwLock<Option<Instant>>>,
    /// Whether a preheat is in progress (avoids duplicate loading).
    preheating: Arc<std::sync::atomic::AtomicBool>,
}

impl EmbeddingService {
    /// Create a new EmbeddingService with the given configuration.
    ///
    /// The model is NOT loaded at this point — use `preheat()` or call `embed()`
    /// to trigger lazy loading.
    pub fn new(config: EmbeddingConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
            config: Arc::new(RwLock::new(config)),
            last_access: Arc::new(RwLock::new(None)),
            preheating: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Get the current configuration.
    pub fn config(&self) -> EmbeddingConfig {
        self.config.read().clone()
    }

    /// Update the configuration.
    ///
    /// If the model name changes, the current model is unloaded (if loaded).
    /// The new model will be lazy-loaded on the next `embed()` call.
    pub fn update_config(&self, new_config: EmbeddingConfig) {
        let model_changed = self.config.read().model != new_config.model;

        *self.config.write() = new_config;

        if model_changed {
            tracing::info!("embedding model changed, unloading current model");
            self.unload();
        }
    }

    /// Set a new model by name.
    ///
    /// This unloads the current model (if any). The new model will be
    /// lazy-loaded on the next access.
    pub fn set_model(&self, model_name: &str) {
        let mut config = self.config.write();
        if config.model != model_name {
            config.model = model_name.to_string();
            drop(config); // release lock before unloading

            tracing::info!(
                model = model_name,
                "switching embedding model, unloading current"
            );
            self.unload();
        }
    }

    /// Check if the model is currently loaded in memory.
    pub fn is_loaded(&self) -> bool {
        self.inner.read().is_some()
    }

    /// Get the model name from config.
    pub fn model_name(&self) -> String {
        self.config.read().model.clone()
    }

    /// Get the embedding dimension.
    ///
    /// Returns `None` if the model is not loaded (dimension is only known after load).
    pub fn dimension(&self) -> Option<usize> {
        self.inner.read().as_ref().map(|e| e.dimension())
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> PathBuf {
        let config = self.config.read();
        resolve_cache_dir(&config.cache_dir)
    }

    /// Preheat — load the model in the background without blocking.
    ///
    /// This spawns a background task that loads the model. Subsequent `embed()`
    /// calls will either use the already-loaded model or wait for preheat to finish.
    pub async fn preheat(&self) {
        // Already loaded?
        if self.inner.read().is_some() {
            return;
        }

        // Already preheating?
        if self
            .preheating
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return; // another task is already preheating
        }

        let inner = self.inner.clone();
        let config = self.config.read().clone();
        let preheating = self.preheating.clone();
        let last_access = self.last_access.clone();

        tracing::info!(model = %config.model, "preheating embedding model in background");

        tokio::task::spawn_blocking(move || {
            let cache_dir = resolve_cache_dir(&config.cache_dir);

            match Embedder::new(&config.model, &cache_dir) {
                Ok(embedder) => {
                    tracing::info!(
                        model = %config.model,
                        dimension = embedder.dimension(),
                        "embedding model preheated successfully"
                    );
                    *inner.write() = Some(embedder);
                    *last_access.write() = Some(Instant::now());
                }
                Err(e) => {
                    tracing::warn!(error = %e, "embedding model preheat failed");
                }
            }

            preheating.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }

    /// Embed a single text.
    ///
    /// Triggers lazy loading if the model is not yet loaded.
    pub async fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        if text.is_empty() {
            return Err(EmbeddingError::EmptyInput.into());
        }

        let embedder = self.ensure_loaded().await?;

        let text_owned = text.to_string();
        let emb = tokio::task::spawn_blocking(move || embedder.embed(&text_owned))
            .await
            .map_err(|e| EmbeddingError::InferenceFailed(format!("task join error: {e}")))??;

        self.touch();
        Ok(emb)
    }

    /// Embed multiple texts in a batch.
    ///
    /// More efficient than calling `embed()` in a loop.
    pub async fn embed_batch(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Err(EmbeddingError::EmptyInput.into());
        }

        let embedder = self.ensure_loaded().await?;

        let texts_owned = texts.to_vec();
        let embeddings = tokio::task::spawn_blocking(move || embedder.embed_batch(&texts_owned))
            .await
            .map_err(|e| EmbeddingError::InferenceFailed(format!("task join error: {e}")))??;

        self.touch();
        Ok(embeddings)
    }

    /// Compute cosine similarity between two texts.
    ///
    /// Embeds both texts and computes their cosine similarity.
    pub async fn similarity(&self, text_a: &str, text_b: &str) -> AppResult<f32> {
        let embeddings = self
            .embed_batch(&[text_a.to_string(), text_b.to_string()])
            .await?;

        if embeddings.len() == 2 {
            Ok(cosine_similarity(&embeddings[0], &embeddings[1]))
        } else {
            Err(EmbeddingError::InferenceFailed("expected 2 embeddings".to_string()).into())
        }
    }

    /// Unload the model from memory, freeing resources.
    ///
    /// The model will be re-loaded on the next `embed()` call (lazy loading).
    pub fn unload(&self) {
        let mut inner = self.inner.write();
        if inner.is_some() {
            tracing::info!("unloading embedding model from memory");
            *inner = None;
            *self.last_access.write() = None;
        }
    }

    /// Check if the model should be unloaded due to idle timeout.
    ///
    /// This should be called periodically (e.g. every 60 seconds) by a background task.
    pub fn check_idle_timeout(&self) {
        let timeout_secs = self.config.read().idle_timeout_secs;
        if timeout_secs == 0 {
            return; // 0 = never unload
        }

        let last_access = self.last_access.read();
        if let Some(last) = *last_access {
            let elapsed = last.elapsed();
            if elapsed > Duration::from_secs(timeout_secs) {
                tracing::info!(
                    idle_secs = elapsed.as_secs(),
                    timeout_secs,
                    "idle timeout reached, unloading embedding model"
                );
                drop(last_access); // release read lock before write
                self.unload();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Ensure the model is loaded, triggering lazy loading if needed.
    ///
    /// Returns an `Arc<Embedder>` that can be used for inference.
    async fn ensure_loaded(&self) -> AppResult<OwnedEmbedder> {
        // Fast path: model is already loaded
        {
            let inner = self.inner.read();
            if inner.is_some() {
                return Ok(OwnedEmbedder {
                    inner: self.inner.clone(),
                });
            }
        }

        // Slow path: need to load the model
        let config = self.config.read().clone();
        let cache_dir = resolve_cache_dir(&config.cache_dir);
        let model_name = config.model.clone();

        tracing::info!(model = %model_name, "lazy-loading embedding model");

        let inner = self.inner.clone();
        let last_access = self.last_access.clone();

        let embedder = tokio::task::spawn_blocking(move || Embedder::new(&model_name, &cache_dir))
            .await
            .map_err(|e| EmbeddingError::InferenceFailed(format!("task join error: {e}")))??;

        // Store the loaded embedder
        *inner.write() = Some(embedder);
        *last_access.write() = Some(Instant::now());

        // Return a handle
        Ok(OwnedEmbedder {
            inner: self.inner.clone(),
        })
    }

    /// Update the last access timestamp.
    fn touch(&self) {
        *self.last_access.write() = Some(Instant::now());
    }
}

impl std::fmt::Debug for EmbeddingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let loaded = self.is_loaded();
        let model = self.model_name();
        f.debug_struct("EmbeddingService")
            .field("model", &model)
            .field("loaded", &loaded)
            .finish()
    }
}

/// A handle to a loaded embedder.
///
/// This struct holds a reference to the shared inner state, allowing
/// the caller to perform inference without holding a write lock.
struct OwnedEmbedder {
    inner: Arc<RwLock<Option<Embedder>>>,
}

impl OwnedEmbedder {
    fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
        let inner = self.inner.read();
        let embedder = inner.as_ref().ok_or(EmbeddingError::ModelNotLoaded)?;
        embedder.embed(text)
    }

    fn embed_batch(&self, texts: &[String]) -> EmbeddingResult<Vec<Vec<f32>>> {
        let inner = self.inner.read();
        let embedder = inner.as_ref().ok_or(EmbeddingError::ModelNotLoaded)?;
        embedder.embed_batch(texts)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> EmbeddingConfig {
        EmbeddingConfig {
            model: "all-MiniLM-L6-v2".to_string(),
            max_batch_size: 32,
            idle_timeout_secs: 0, // never unload in tests
            cache_dir: String::new(),
            high_threshold: 0.82,
            low_threshold: 0.65,
        }
    }

    #[test]
    fn test_service_creation() {
        let service = EmbeddingService::new(make_config());
        assert!(!service.is_loaded());
        assert_eq!(service.model_name(), "all-MiniLM-L6-v2");
        assert!(service.dimension().is_none());
    }

    #[test]
    fn test_set_model_same() {
        let service = EmbeddingService::new(make_config());
        service.set_model("all-MiniLM-L6-v2");
        assert_eq!(service.model_name(), "all-MiniLM-L6-v2");
    }

    #[test]
    fn test_set_model_different() {
        let service = EmbeddingService::new(make_config());
        service.set_model("bge-small-zh-v1.5");
        assert_eq!(service.model_name(), "bge-small-zh-v1.5");
    }

    #[test]
    fn test_update_config() {
        let service = EmbeddingService::new(make_config());

        let mut new_config = make_config();
        new_config.model = "bge-small-en-v1.5".to_string();
        new_config.high_threshold = 0.90;

        service.update_config(new_config);

        assert_eq!(service.model_name(), "bge-small-en-v1.5");
        assert_eq!(service.config().high_threshold, 0.90);
    }

    #[test]
    fn test_unload_when_not_loaded() {
        let service = EmbeddingService::new(make_config());
        // Should be a no-op
        service.unload();
        assert!(!service.is_loaded());
    }

    #[test]
    fn test_check_idle_timeout_no_timeout() {
        let service = EmbeddingService::new(make_config());
        // idle_timeout_secs = 0 means never unload
        service.check_idle_timeout();
        // Should not crash
    }

    #[test]
    fn test_check_idle_timeout_with_timeout() {
        let mut config = make_config();
        config.idle_timeout_secs = 1; // 1 second timeout

        let service = EmbeddingService::new(config);

        // Set last access to 5 seconds ago
        *service.last_access.write() = Some(Instant::now() - Duration::from_secs(5));

        service.check_idle_timeout();
        // Model wasn't loaded to begin with, but check_idle should handle it gracefully
    }

    #[tokio::test]
    async fn test_embed_empty_text() {
        let service = EmbeddingService::new(make_config());
        let result = service.embed("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_embed_batch_empty() {
        let service = EmbeddingService::new(make_config());
        let result = service.embed_batch(&[]).await;
        assert!(result.is_err());
    }

    // ── Ignored model-loading tests ──────────────────────────────────────────
    // Root cause is NOT "no network": HF direct and the hf-mirror Xet CDN
    // (`*.cdn.hf.co`) are blocked for model *weights* (metadata is reachable).
    // ONNX weights are obtainable from ModelScope and staged into the HF cache.
    //
    // UNBLOCK (verified 2026-08-10): stage all-MiniLM-L6-v2 ONNX via ModelScope,
    // then run with:  HF_HUB_CACHE=<cache> HF_HOME=<cache> cargo test --lib -- --ignored
    // The 5 MiniLM tests below then PASS (9 MiniLM tests pass project-wide).
    //
    // CANNOT UNBLOCK: `test_service_model_switch` switches to bge-small-zh / -en,
    // which have NO ONNX on ModelScope (safetensors/pytorch only) and are
    // unreachable via HF/Xet CDN. It stays ignored until an ONNX bge source exists.
    // Ref: IGNORED_TESTS_AUDIT.md, P20_SIMILARITY_THRESHOLD_MEASUREMENT.md

    #[tokio::test]
    #[ignore]
    async fn test_service_lazy_load_and_embed() {
        let service = EmbeddingService::new(make_config());
        assert!(!service.is_loaded());

        let emb = service.embed("hello world").await.unwrap();
        assert_eq!(emb.len(), 384);
        assert!(service.is_loaded());
        assert_eq!(service.dimension(), Some(384));
    }

    #[tokio::test]
    #[ignore]
    async fn test_service_preheat() {
        let service = EmbeddingService::new(make_config());
        assert!(!service.is_loaded());

        service.preheat().await;

        // Wait for background loading to complete
        for _ in 0..30 {
            if service.is_loaded() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        assert!(service.is_loaded());

        // Embed should work immediately (no cold start)
        let emb = service.embed("test").await.unwrap();
        assert_eq!(emb.len(), 384);
    }

    #[tokio::test]
    #[ignore]
    async fn test_service_unload_and_reload() {
        let service = EmbeddingService::new(make_config());

        let _ = service.embed("hello").await.unwrap();
        assert!(service.is_loaded());

        service.unload();
        assert!(!service.is_loaded());

        // Re-embed should trigger lazy reload
        let _ = service.embed("hello").await.unwrap();
        assert!(service.is_loaded());
    }

    #[tokio::test]
    #[ignore]
    async fn test_service_batch_embed() {
        let service = EmbeddingService::new(make_config());

        let texts: Vec<String> = (0..10).map(|i| format!("test sentence {i}")).collect();
        let embeddings = service.embed_batch(&texts).await.unwrap();

        assert_eq!(embeddings.len(), 10);
        for emb in &embeddings {
            assert_eq!(emb.len(), 384);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_service_similarity() {
        let service = EmbeddingService::new(make_config());

        let sim_same = service.similarity("hello", "hello").await.unwrap();
        assert!((sim_same - 1.0).abs() < 1e-4, "identical texts: {sim_same}");

        let sim_diff = service
            .similarity("hello world", "quantum mechanics")
            .await
            .unwrap();
        assert!(sim_diff < 0.5, "unrelated texts: {sim_diff}");
    }

    // switches to bge (no ONNX on ModelScope, HF/Xet CDN blocked) → cannot unblock.
    #[tokio::test]
    #[ignore]
    async fn test_service_model_switch() {
        let service = EmbeddingService::new(make_config());

        // Load all-MiniLM-L6-v2 (384 dim)
        let _ = service.embed("hello").await.unwrap();
        assert_eq!(service.dimension(), Some(384));

        // Switch to bge-small-zh-v1.5 (512 dim)
        service.set_model("bge-small-zh-v1.5");
        assert!(!service.is_loaded()); // unloaded

        let _ = service.embed("你好").await.unwrap();
        assert_eq!(service.dimension(), Some(512));
    }
}
