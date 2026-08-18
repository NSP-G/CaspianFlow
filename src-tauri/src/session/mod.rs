//! P21 — local SQLite session management.
//!
//! Structured persistence base for all CaspianFlow interactions: every
//! message, workflow run and agent call hangs off a `session_id` (design
//! principle #1 — session is the aggregate root). The SQLite backend is the
//! default, not the only option: the [`SessionStore`] trait is backend-agnostic
//! (design principle #3).
//!
//! ## Relationship to other phases
//! - **P17 `RunStore`**: file-based, ephemeral (`temp/`), step-level debug detail.
//!   This module is persistent, session-level, user-visible. Two parallel
//!   pipelines, not overlapping (see `P21_PRECHECK.md` gap G4).
//! - **P20 cache**: stores *execution results* (same input → reuse output). This
//!   module stores *interaction history* (who/when/what/result). The
//!   `WorkflowRunRecord.cache_hit` flag records the intersection.

pub mod error;
pub mod schema;
pub mod store;
pub mod types;

pub use error::{SessionError, SessionResult};
pub use schema::CURRENT_SCHEMA;
pub use store::{SessionStore, SqliteSessionStore};
pub use types::{
    AgentCallRecord, Message, MessageRole, Session, SessionStatus, WorkflowRunRecord,
    WorkflowRunStatus,
};
