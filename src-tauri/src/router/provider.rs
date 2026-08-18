//! Embedding provider abstraction.
//!
//! The `EmbeddingProvider` trait decouples the router from any specific
//! embedding implementation. This enables:
//!
//! - **Production**: `EmbeddingServiceAdapter` wraps the real `EmbeddingService` (P11)
//! - **Testing**: `MockEmbeddingProvider` generates deterministic pseudo-vectors
//!   without any network or model download
//!
//! ## Mock strategy
//!
//! The mock provider generates vectors based on character-level features of the
//! input text. Texts that share characters (especially Chinese characters or
//! key English words) will produce higher cosine similarity, which allows
//! the router tests to exercise the full routing pipeline without a real model.

use async_trait::async_trait;

use crate::knowledge::similarity::l2_normalize;
use crate::types::{AppError, AppResult, EmbeddingError};

/// Abstraction over any embedding source.
///
/// Implementations must be `Send + Sync` so they can be shared across async tasks.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text into a vector.
    async fn embed(&self, text: &str) -> AppResult<Vec<f32>>;

    /// Embed multiple texts in a batch (more efficient than calling `embed` in a loop).
    async fn embed_batch(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>>;

    /// Return the embedding dimension, if known.
    fn dimension(&self) -> Option<usize>;
}

// ---------------------------------------------------------------------------
// Adapter for the real EmbeddingService
// ---------------------------------------------------------------------------

/// Adapter that wraps `EmbeddingService` to implement `EmbeddingProvider`.
///
/// This allows the router to use the real embedding model without depending
/// on `EmbeddingService` directly.
pub struct EmbeddingServiceAdapter {
    service: std::sync::Arc<crate::knowledge::EmbeddingService>,
}

impl EmbeddingServiceAdapter {
    pub fn new(service: std::sync::Arc<crate::knowledge::EmbeddingService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EmbeddingProvider for EmbeddingServiceAdapter {
    async fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        self.service.embed(text).await
    }

    async fn embed_batch(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        self.service.embed_batch(texts).await
    }

    fn dimension(&self) -> Option<usize> {
        self.service.dimension()
    }
}

// ---------------------------------------------------------------------------
// Mock provider
// ---------------------------------------------------------------------------

/// A mock embedding provider for testing.
///
/// Generates deterministic pseudo-vectors based on text content.
/// Texts with overlapping characters produce higher similarity.
pub struct MockEmbeddingProvider {
    dimension: usize,
}

impl MockEmbeddingProvider {
    /// Create a new mock provider with the given vector dimension.
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    /// Generate a deterministic pseudo-embedding for the given text.
    ///
    /// The strategy maps each character to a vector slot and accumulates
    /// a value. Texts that share characters will have overlapping non-zero
    /// slots, yielding higher cosine similarity.
    fn pseudo_embed(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0_f32; self.dimension];

        for ch in text.chars() {
            let idx = (ch as usize) % self.dimension;
            vec[idx] += 1.0;
        }

        // Also add bigram features for better discrimination
        let chars: Vec<char> = text.chars().collect();
        for window in chars.windows(2) {
            let combined = (window[0] as usize) * 31 + (window[1] as usize);
            let idx = combined % self.dimension;
            vec[idx] += 0.5;
        }

        l2_normalize(&mut vec);
        vec
    }

    /// Public accessor for test code that needs pre-computed embeddings.
    #[cfg(test)]
    pub fn pseudo_embed_public(&self, text: &str) -> Vec<f32> {
        self.pseudo_embed(text)
    }
}

#[async_trait]
impl EmbeddingProvider for MockEmbeddingProvider {
    async fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        if text.is_empty() {
            return Err(AppError::Embedding(EmbeddingError::EmptyInput));
        }
        Ok(self.pseudo_embed(text))
    }

    async fn embed_batch(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Err(AppError::Embedding(EmbeddingError::EmptyInput));
        }
        Ok(texts.iter().map(|t| self.pseudo_embed(t)).collect())
    }

    fn dimension(&self) -> Option<usize> {
        Some(self.dimension)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider() -> MockEmbeddingProvider {
        MockEmbeddingProvider::new(128)
    }

    #[tokio::test]
    async fn test_mock_embed_basic() {
        let provider = make_provider();
        let emb = provider.embed("hello world").await.unwrap();
        assert_eq!(emb.len(), 128);

        // Should be normalized (unit norm)
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn test_mock_embed_deterministic() {
        let provider = make_provider();
        let emb1 = provider.embed("test text").await.unwrap();
        let emb2 = provider.embed("test text").await.unwrap();
        assert_eq!(emb1, emb2);
    }

    #[tokio::test]
    async fn test_mock_embed_empty() {
        let provider = make_provider();
        let result = provider.embed("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_batch() {
        let provider = make_provider();
        let texts = vec!["hello".to_string(), "world".to_string(), "test".to_string()];
        let embeddings = provider.embed_batch(&texts).await.unwrap();
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), 128);
        }
    }

    #[tokio::test]
    async fn test_mock_batch_empty() {
        let provider = make_provider();
        let result = provider.embed_batch(&[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_dimension() {
        let provider = MockEmbeddingProvider::new(256);
        assert_eq!(provider.dimension(), Some(256));
    }

    #[tokio::test]
    async fn test_similar_texts_high_similarity() {
        let provider = make_provider();

        let emb1 = provider.embed("读取文件").await.unwrap();
        let emb2 = provider.embed("读取这个文件").await.unwrap();

        let sim = crate::knowledge::similarity::cosine_similarity(&emb1, &emb2);
        // Texts sharing most characters should have high similarity
        assert!(
            sim > 0.5,
            "similar texts should have high similarity, got {sim}"
        );
    }

    #[tokio::test]
    async fn test_dissimilar_texts_low_similarity() {
        let provider = make_provider();

        let emb1 = provider.embed("读取文件").await.unwrap();
        let emb2 = provider.embed("weather forecast today").await.unwrap();

        let sim = crate::knowledge::similarity::cosine_similarity(&emb1, &emb2);
        // Texts with no shared characters should have very low similarity
        assert!(
            sim < 0.3,
            "dissimilar texts should have low similarity, got {sim}"
        );
    }
}
