//! Shared type definitions.

pub mod error;

pub use error::{
    AppError, AppResult, ConfigError, ConfigResult, EmbeddingError, EmbeddingResult, ExecutorError,
    ExecutorResult, GuardianError, GuardianResult, LlmError, LlmResult, SkillError, SkillResult,
    SlotFillingError, SlotFillingResult, WorkflowError, WorkflowResult,
};
