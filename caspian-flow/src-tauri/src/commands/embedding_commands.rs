//! Embedding IPC commands.
//!
//! These functions are designed to be annotated with `#[tauri::command]`
//! once the Tauri runtime is integrated. For now they are plain async
//! functions that can be called from Rust or wrapped by the frontend bridge.

use crate::config::settings::EmbeddingConfig;
use crate::knowledge::EmbeddingService;
use crate::types::AppResult;

/// Embed a single text and return the vector.
pub async fn embed_text(service: &EmbeddingService, text: &str) -> AppResult<Vec<f32>> {
    service.embed(text).await
}

/// Embed multiple texts in a batch.
pub async fn embed_batch(
    service: &EmbeddingService,
    texts: Vec<String>,
) -> AppResult<Vec<Vec<f32>>> {
    service.embed_batch(&texts).await
}

/// Compute cosine similarity between two texts.
pub async fn compute_similarity(
    service: &EmbeddingService,
    text_a: &str,
    text_b: &str,
) -> AppResult<f32> {
    service.similarity(text_a, text_b).await
}

/// Preheat the embedding model in the background.
pub async fn preload_model(service: &EmbeddingService) -> AppResult<()> {
    service.preheat().await;
    Ok(())
}

/// Unload the embedding model from memory.
pub fn unload_model(service: &EmbeddingService) -> AppResult<()> {
    service.unload();
    Ok(())
}

/// Check if the model is currently loaded.
pub fn is_model_loaded(service: &EmbeddingService) -> AppResult<bool> {
    Ok(service.is_loaded())
}

/// Get the current model name.
pub fn get_model_name(service: &EmbeddingService) -> AppResult<String> {
    Ok(service.model_name())
}

/// Get the embedding dimension (if model is loaded).
pub fn get_dimension(service: &EmbeddingService) -> AppResult<Option<usize>> {
    Ok(service.dimension())
}

/// Get the full embedding configuration.
pub fn get_embedding_config(service: &EmbeddingService) -> AppResult<EmbeddingConfig> {
    Ok(service.config())
}

/// Switch to a different embedding model.
pub fn set_model(service: &EmbeddingService, model_name: &str) -> AppResult<()> {
    service.set_model(model_name);
    Ok(())
}

/// Update the full embedding configuration.
pub fn update_embedding_config(
    service: &EmbeddingService,
    config: EmbeddingConfig,
) -> AppResult<()> {
    service.update_config(config);
    Ok(())
}

/// Check idle timeout and unload if expired.
pub fn check_idle(service: &EmbeddingService) -> AppResult<()> {
    service.check_idle_timeout();
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service() -> EmbeddingService {
        EmbeddingService::new(EmbeddingConfig {
            model: "all-MiniLM-L6-v2".to_string(),
            max_batch_size: 32,
            idle_timeout_secs: 0,
            cache_dir: String::new(),
            high_threshold: 0.82,
            low_threshold: 0.65,
        })
    }

    #[tokio::test]
    async fn test_embed_empty_text_errors() {
        let service = make_service();
        let result = embed_text(&service, "").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_embed_batch_empty_errors() {
        let service = make_service();
        let result = embed_batch(&service, vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_is_model_loaded() {
        let service = make_service();
        assert!(!is_model_loaded(&service).unwrap());
    }

    #[tokio::test]
    async fn test_get_model_name() {
        let service = make_service();
        assert_eq!(get_model_name(&service).unwrap(), "all-MiniLM-L6-v2");
    }

    #[tokio::test]
    async fn test_get_dimension_none() {
        let service = make_service();
        assert_eq!(get_dimension(&service).unwrap(), None);
    }

    #[tokio::test]
    async fn test_get_embedding_config() {
        let service = make_service();
        let config = get_embedding_config(&service).unwrap();
        assert_eq!(config.model, "all-MiniLM-L6-v2");
        assert_eq!(config.high_threshold, 0.82);
    }

    #[tokio::test]
    async fn test_set_model() {
        let service = make_service();
        set_model(&service, "bge-small-zh-v1.5").unwrap();
        assert_eq!(get_model_name(&service).unwrap(), "bge-small-zh-v1.5");
    }

    #[tokio::test]
    async fn test_update_embedding_config() {
        let service = make_service();

        let new_config = EmbeddingConfig {
            model: "bge-small-en-v1.5".to_string(),
            max_batch_size: 64,
            idle_timeout_secs: 300,
            cache_dir: String::new(),
            high_threshold: 0.90,
            low_threshold: 0.70,
        };

        update_embedding_config(&service, new_config).unwrap();
        let config = get_embedding_config(&service).unwrap();
        assert_eq!(config.model, "bge-small-en-v1.5");
        assert_eq!(config.max_batch_size, 64);
        assert_eq!(config.high_threshold, 0.90);
    }

    #[tokio::test]
    async fn test_unload_model() {
        let service = make_service();
        unload_model(&service).unwrap();
        assert!(!is_model_loaded(&service).unwrap());
    }

    #[tokio::test]
    async fn test_preload_model() {
        let service = make_service();
        preload_model(&service).await.unwrap();
        // Preheat is async — just verify it doesn't crash
    }

    #[tokio::test]
    async fn test_check_idle() {
        let service = make_service();
        check_idle(&service).unwrap();
    }
}
