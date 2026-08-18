//! Embedder — the core embedding model wrapper built on `fastembed`.
//!
//! This module provides a synchronous API around fastembed's `TextEmbedding`.
//! The `EmbeddingService` in `mod.rs` wraps this with async + lazy-loading.

use std::path::Path;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::types::{EmbeddingError, EmbeddingResult};

use super::model_download;

/// Supported embedding models with their fastembed mappings.
///
/// The string identifiers match what users configure in `settings.yaml`:
///   `embedding.model: "bge-small-zh-v1.5"`
pub fn parse_model(name: &str) -> EmbeddingResult<EmbeddingModel> {
    match name {
        "bge-small-zh-v1.5" => Ok(EmbeddingModel::BGESmallZHV15),
        "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
        "bge-large-zh-v1.5" => Ok(EmbeddingModel::BGELargeZHV15),
        "bge-large-en-v1.5" => Ok(EmbeddingModel::BGELargeENV15),
        "all-MiniLM-L6-v2" => Ok(EmbeddingModel::AllMiniLML6V2),
        "all-MiniLM-L12-v2" => Ok(EmbeddingModel::AllMiniLML12V2),
        "multilingual-e5-small" => Ok(EmbeddingModel::MultilingualE5Small),
        _ => Err(EmbeddingError::UnsupportedModel {
            model: name.to_string(),
        }),
    }
}

/// Get the dimension (vector length) for a given model.
///
/// These are known constant dimensions per model architecture.
pub fn model_dimension(model: &EmbeddingModel) -> usize {
    match model {
        EmbeddingModel::BGESmallZHV15 => 512,
        EmbeddingModel::BGESmallENV15 => 384,
        EmbeddingModel::BGELargeZHV15 => 1024,
        EmbeddingModel::BGELargeENV15 => 1024,
        EmbeddingModel::AllMiniLML6V2 => 384,
        EmbeddingModel::AllMiniLML12V2 => 384,
        EmbeddingModel::MultilingualE5Small => 384,
        // For other models, fastembed provides this via the model info
        _ => {
            // Query fastembed's model info
            TextEmbedding::list_supported_models()
                .iter()
                .find(|info| &info.model == model)
                .map(|info| info.dim)
                .unwrap_or(384) // safe fallback
        }
    }
}

/// The core Embedder — owns a loaded `TextEmbedding` model.
///
/// This is a synchronous, CPU-bound struct. It is `Send` + `Sync` so it can
/// be shared across threads (via `Arc` or behind a `RwLock`).
pub struct Embedder {
    model: TextEmbedding,
    model_name: String,
    dimension: usize,
}

impl Embedder {
    /// Create a new Embedder by loading the specified model.
    ///
    /// This will download the model on first use (cached in `cache_dir`).
    /// If offline mode is detected and the model is not cached, returns an error.
    ///
    /// # Arguments
    /// * `model_name` - The model identifier (e.g. "bge-small-zh-v1.5")
    /// * `cache_dir` - Directory for model file caching
    pub fn new(model_name: &str, cache_dir: &Path) -> EmbeddingResult<Self> {
        let model = parse_model(model_name)?;
        let dim = model_dimension(&model);

        // Check offline mode
        let offline = model_download::is_offline_mode();
        let cached = model_download::is_model_cached(model_name, cache_dir);

        if offline && !cached {
            return Err(EmbeddingError::OfflineModelNotFound {
                model: model_name.to_string(),
                cache_dir: cache_dir.display().to_string(),
            });
        }

        tracing::info!(
            model = model_name,
            dimension = dim,
            cache_dir = %cache_dir.display(),
            offline,
            cached,
            "initializing embedder"
        );

        // Initialize with retry logic
        let cache_dir_owned = cache_dir.to_path_buf();
        let model_clone = model.clone();

        let text_embedding = model_download::init_with_retry(
            || {
                let opts = InitOptions::new(model_clone.clone())
                    .with_cache_dir(cache_dir_owned.clone())
                    .with_show_download_progress(true);

                TextEmbedding::try_new(opts).map_err(|e| e.to_string())
            },
            None,
        )?;

        Ok(Self {
            model: text_embedding,
            model_name: model_name.to_string(),
            dimension: dim,
        })
    }

    /// Embed a single text string.
    ///
    /// Returns a vector of `f32` values with length `self.dimension()`.
    pub fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
        if text.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }

        let embeddings = self
            .model
            .embed(vec![text], None)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::InferenceFailed("no embedding returned".to_string()))
    }

    /// Embed multiple texts in a batch.
    ///
    /// More efficient than calling `embed()` in a loop — fastembed processes
    /// all texts in a single forward pass.
    ///
    /// # Arguments
    /// * `texts` - The texts to embed (must be non-empty)
    ///
    /// # Returns
    /// A vector of embedding vectors, one per input text.
    pub fn embed_batch(&self, texts: &[String]) -> EmbeddingResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }

        let docs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

        let embeddings = self
            .model
            .embed(docs, None)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;

        // Validate dimensions
        for emb in &embeddings {
            if emb.len() != self.dimension {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.dimension,
                    actual: emb.len(),
                });
            }
        }

        Ok(embeddings)
    }

    /// Get the embedding dimension for this model.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Get the model name.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }
}

impl std::fmt::Debug for Embedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Embedder")
            .field("model_name", &self.model_name)
            .field("dimension", &self.dimension)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_model_supported() {
        assert!(parse_model("bge-small-zh-v1.5").is_ok());
        assert!(parse_model("bge-small-en-v1.5").is_ok());
        assert!(parse_model("all-MiniLM-L6-v2").is_ok());
        assert!(parse_model("multilingual-e5-small").is_ok());
    }

    #[test]
    fn test_parse_model_unsupported() {
        let result = parse_model("nonexistent-model");
        assert!(result.is_err());
        match result.unwrap_err() {
            EmbeddingError::UnsupportedModel { model } => {
                assert_eq!(model, "nonexistent-model");
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
    }

    #[test]
    fn test_model_dimension() {
        assert_eq!(model_dimension(&EmbeddingModel::BGESmallZHV15), 512);
        assert_eq!(model_dimension(&EmbeddingModel::BGESmallENV15), 384);
        assert_eq!(model_dimension(&EmbeddingModel::AllMiniLML6V2), 384);
        assert_eq!(model_dimension(&EmbeddingModel::MultilingualE5Small), 384);
    }

    #[test]
    fn test_parse_and_dimension_consistency() {
        let model = parse_model("bge-small-zh-v1.5").unwrap();
        assert_eq!(model_dimension(&model), 512);

        let model = parse_model("bge-small-en-v1.5").unwrap();
        assert_eq!(model_dimension(&model), 384);
    }

    // ── Ignored model-loading tests ──────────────────────────────────────────
    // Root cause is NOT "no network": HF direct and the hf-mirror Xet CDN
    // (`*.cdn.hf.co`) are blocked for model *weights* (metadata is reachable).
    // ONNX weights are obtainable from ModelScope and staged into the HF cache.
    //
    // UNBLOCK (verified 2026-08-10): stage all-MiniLM-L6-v2 ONNX via ModelScope,
    // then run with:  HF_HUB_CACHE=<cache> HF_HOME=<cache> cargo test --lib -- --ignored
    // The 4 MiniLM tests below then PASS (9 MiniLM tests pass project-wide).
    //
    // CANNOT UNBLOCK: `test_embedder_bge_zh` uses bge-small-zh-v1.5, which has NO
    // ONNX on ModelScope (safetensors/pytorch only) and is unreachable via HF/Xet
    // CDN. It stays ignored until an ONNX bge source exists.
    // Ref: IGNORED_TESTS_AUDIT.md, P20_SIMILARITY_THRESHOLD_MEASUREMENT.md

    #[tokio::test]
    #[ignore]
    async fn test_embedder_new_and_embed() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("models");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let embedder = Embedder::new("all-MiniLM-L6-v2", &cache_dir).unwrap();
        assert_eq!(embedder.dimension(), 384);
        assert_eq!(embedder.model_name(), "all-MiniLM-L6-v2");

        let emb = embedder.embed("hello world").unwrap();
        assert_eq!(emb.len(), 384);

        // Verify the embedding is not all zeros
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(norm > 0.0, "embedding should not be zero vector");
    }

    #[tokio::test]
    #[ignore]
    async fn test_embedder_batch() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("models");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let embedder = Embedder::new("all-MiniLM-L6-v2", &cache_dir).unwrap();

        let texts = vec![
            "hello world".to_string(),
            "goodbye world".to_string(),
            "the quick brown fox".to_string(),
        ];

        let embeddings = embedder.embed_batch(&texts).unwrap();
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), 384);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_embedder_empty_input() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("models");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let embedder = Embedder::new("all-MiniLM-L6-v2", &cache_dir).unwrap();

        assert!(embedder.embed("").is_err());
        assert!(embedder.embed_batch(&[]).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn test_embedder_similarity() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("models");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let embedder = Embedder::new("all-MiniLM-L6-v2", &cache_dir).unwrap();

        let emb1 = embedder.embed("hello world").unwrap();
        let emb2 = embedder.embed("hello world").unwrap();
        let emb3 = embedder.embed("quantum physics equations").unwrap();

        let sim_same = super::super::similarity::cosine_similarity(&emb1, &emb2);
        let sim_diff = super::super::similarity::cosine_similarity(&emb1, &emb3);

        assert!(
            (sim_same - 1.0).abs() < 1e-4,
            "identical texts should have similarity ~1.0, got {sim_same}"
        );
        assert!(
            sim_diff < 0.5,
            "unrelated texts should have low similarity, got {sim_diff}"
        );
    }

    // bge-small-zh-v1.5 has no ONNX on ModelScope + HF/Xet CDN blocked → cannot unblock.
    #[tokio::test]
    #[ignore]
    async fn test_embedder_bge_zh() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("models");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let embedder = Embedder::new("bge-small-zh-v1.5", &cache_dir).unwrap();
        assert_eq!(embedder.dimension(), 512);

        let emb = embedder.embed("你好世界").unwrap();
        assert_eq!(emb.len(), 512);
    }
}
