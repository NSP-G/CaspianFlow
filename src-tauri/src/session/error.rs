//! Session-store errors (P21).
//!
//! Follows the crate convention: each domain owns a `*Error` type, and
//! `crate::types::error::AppError` aggregates them via `#[from]`.

use thiserror::Error;

/// Errors produced by the session store.
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("session not found: `{0}`")]
    NotFound(String),

    #[error("schema migration failed at version {version}: {reason}")]
    Migration { version: u32, reason: String },

    #[error("database integrity check failed: {0}")]
    Integrity(String),

    #[error("backup failed: {0}")]
    Backup(String),
}

pub type SessionResult<T> = Result<T, SessionError>;
