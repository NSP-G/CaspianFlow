//! Knowledge-base (P22) errors.
//!
//! Follows the crate convention: each domain owns a `*Error` type, and
//! `crate::types::error::AppError` aggregates them via `#[from]`.

use thiserror::Error;

/// Errors produced by the knowledge base (import / retrieval / QA).
#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("document not found: `{0}`")]
    NotFound(String),

    #[error("schema migration failed at version {version}: {reason}")]
    Migration { version: u32, reason: String },

    #[error("database integrity check failed: {0}")]
    Integrity(String),

    #[error("LLM error during answer generation: {0}")]
    Llm(#[from] crate::types::LlmError),

    /// The LLM returned an empty or whitespace-only answer.
    #[error("LLM returned an empty answer")]
    EmptyAnswer,

    /// The source document had no extractable text content.
    #[error("document content is empty")]
    EmptyContent,

    /// Embedding (vectorization) failed — e.g. model unavailable or OOM.
    #[error("embedding error: {0}")]
    Embedding(String),

    /// Model selection/routing failed (P24): no model available after the
    /// priority + fallback chain, or an unsupported selection strategy.
    #[error("model selection failed: {0}")]
    ModelSelection(#[from] crate::router::ModelRouterError),
}

pub type KnowledgeResult<T> = Result<T, KnowledgeError>;
