//! Unified error types for CaspianFlow.

use thiserror::Error;

use crate::knowledge::KnowledgeError;
use crate::session::SessionError;

/// Top-level application error.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    #[error("session error: {0}")]
    Session(#[from] SessionError),

    #[error("knowledge base error: {0}")]
    Knowledge(#[from] KnowledgeError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),

    #[error("skill error: {0}")]
    Skill(#[from] SkillError),

    #[error("embedding error: {0}")]
    Embedding(#[from] EmbeddingError),

    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    #[error("slot filling error: {0}")]
    SlotFilling(#[from] SlotFillingError),

    #[error("guardian error: {0}")]
    Guardian(#[from] GuardianError),

    #[error("executor error: {0}")]
    Executor(#[from] ExecutorError),

    #[error("workflow error: {0}")]
    Workflow(#[from] WorkflowError),
}

/// Configuration-specific errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required field: `{field}` in {location}")]
    MissingField { field: String, location: String },

    #[error("invalid value for `{field}`: {reason}")]
    InvalidValue { field: String, reason: String },

    #[error("config file not found: {path}")]
    FileNotFound { path: String },

    #[error("config parse error: {0}")]
    Parse(String),

    #[error("schema version `{actual}` is not supported (expected `{expected}`)")]
    UnsupportedSchema { actual: String, expected: String },

    #[error("migration failed at version `{version}`: {reason}")]
    Migration { version: String, reason: String },

    #[error("environment variable not set: `{var}`")]
    EnvVarMissing { var: String },

    #[error("duplicate model id: `{id}`")]
    DuplicateModelId { id: String },
}

/// Skill-specific errors.
#[derive(Debug, Error)]
pub enum SkillError {
    #[error("skill not found: `{name}`")]
    NotFound { name: String },

    #[error("skill manifest parse error in {path}: {reason}")]
    ParseError { path: String, reason: String },

    #[error("skill validation error in {path}: {errors}")]
    ValidationError { path: String, errors: String },

    #[error("skill manifest not found: {path}")]
    ManifestNotFound { path: String },

    #[error("skill scan error: {0}")]
    ScanError(String),
}

/// LLM (large language model) call errors.
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("network error during LLM call: {0}")]
    NetworkError(String),

    #[error("rate limited by LLM provider")]
    RateLimited,

    #[error("LLM returned invalid response: {0}")]
    InvalidResponse(String),

    #[error("LLM call timed out")]
    Timeout,

    #[error("LLM provider not configured")]
    NotConfigured,
}

/// Slot-filling (parameter extraction) errors.
#[derive(Debug, Error)]
pub enum SlotFillingError {
    #[error("JSON parse failed — could not extract valid JSON from LLM output: {raw_output}")]
    JsonParseFailed { raw_output: String },

    #[error("schema validation failed with {error_count} error(s): {errors}")]
    ValidationFailed {
        error_count: usize,
        errors: String,
        raw_output: String,
    },

    #[error("max retries ({max_retries}) exceeded — extraction failed after {attempts} attempts")]
    MaxRetriesExceeded {
        max_retries: usize,
        attempts: usize,
        last_errors: Vec<String>,
        raw_output: String,
    },

    #[error("missing required fields: {fields}")]
    MissingRequiredFields {
        fields: String,
        partial_params: serde_json::Value,
    },

    #[error("LLM error during slot filling: {0}")]
    LlmError(#[from] LlmError),

    #[error("empty input schema — cannot fill slots without a schema")]
    EmptySchema,

    #[error("unsupported schema keyword `{keyword}` — supported keywords: type, required, properties, minimum, maximum, minLength, maxLength, enum, items, default")]
    UnsupportedSchemaKeyword { keyword: String },
}

/// Guardian (output validation) errors.
#[derive(Debug, Error)]
pub enum GuardianError {
    #[error("L1 format check failed — output is not valid JSON: {raw_output}")]
    FormatCheckFailed { raw_output: String },

    #[error("L2 schema validation failed with {error_count} error(s): {errors}")]
    SchemaValidationFailed {
        error_count: usize,
        errors: String,
        raw_output: String,
    },

    #[error("max retries ({max_retries}) exceeded — validation failed after {attempts} attempts, last errors: {last_errors:?}")]
    MaxRetriesExceeded {
        max_retries: usize,
        attempts: usize,
        last_errors: Vec<String>,
        raw_output: String,
    },

    #[error("L4 security check blocked output — {violations:?}")]
    SecurityBlocked { violations: Vec<String> },

    #[error("output size {actual} exceeds limit {limit} bytes")]
    OutputTooLarge { actual: usize, limit: usize },

    #[error("LLM error during guardian retry: {0}")]
    LlmError(#[from] LlmError),

    #[error("empty output schema — cannot validate without a schema")]
    EmptySchema,
}

/// Executor (skill subprocess execution) errors.
#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("runtime `{runtime}` not found: {reason}")]
    RuntimeNotFound { runtime: String, reason: String },

    #[error("skill `{skill_name}` execution timed out after {timeout_secs}s")]
    Timeout {
        skill_name: String,
        timeout_secs: u64,
    },

    #[error("skill `{skill_name}` exited with code {exit_code}")]
    NonZeroExitCode {
        skill_name: String,
        exit_code: i32,
        stdout: String,
        stderr: String,
    },

    #[error("skill entry file not found: {path}")]
    EntryNotFound { path: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("input serialization failed: {0}")]
    InputSerialization(String),

    #[error("skill execution failed: {0}")]
    ExecutionFailed(String),

    #[error("execution pool exhausted (max concurrent: {max_concurrent})")]
    PoolExhausted { max_concurrent: usize },

    #[error("memory limit setup failed: {reason}")]
    MemoryLimitFailed { reason: String },

    #[error("permission denied for skill `{skill_name}`: {reason}")]
    PermissionDenied { skill_name: String, reason: String },
}

/// Embedding-specific errors.
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("unsupported embedding model: `{model}`")]
    UnsupportedModel { model: String },

    #[error("embedding model not loaded — call preheat() or embed() to trigger lazy init")]
    ModelNotLoaded,

    #[error("model init failed after {retries} retries: {reason}")]
    ModelInitFailed { retries: usize, reason: String },

    #[error("embedding inference failed: {0}")]
    InferenceFailed(String),

    #[error("empty input — cannot embed zero texts")]
    EmptyInput,

    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("offline mode: model `{model}` not found in cache dir `{cache_dir}` — place model files manually or connect to network")]
    OfflineModelNotFound { model: String, cache_dir: String },
}

/// Workflow engine errors.
#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("workflow not found: `{name}`")]
    NotFound { name: String },

    #[error("workflow manifest parse error in {path}: {reason}")]
    ParseError { path: String, reason: String },

    #[error("workflow validation error in `{workflow_name}`: {errors}")]
    ValidationError {
        workflow_name: String,
        errors: String,
    },

    #[error("workflow manifest not found: {path}")]
    ManifestNotFound { path: String },

    #[error("workflow scan error: {0}")]
    ScanError(String),

    /// Explicit save aborted because the formal file was modified externally
    /// since it was loaded (mtime mismatch). P27 冲突检测 (验收 #5).
    #[error("workflow `{name}` conflict: {reason}")]
    Conflict { name: String, reason: String },

    #[error("step `{step_id}` failed: {reason}")]
    StepFailed { step_id: String, reason: String },

    #[error("step `{step_id}` not found in workflow `{workflow_name}`")]
    StepNotFound {
        step_id: String,
        workflow_name: String,
    },

    #[error("duplicate step id `{step_id}` in workflow `{workflow_name}`")]
    DuplicateStepId {
        step_id: String,
        workflow_name: String,
    },

    #[error("cycle detected in workflow `{workflow_name}`: {cycle}")]
    CycleDetected {
        workflow_name: String,
        cycle: String,
    },

    #[error("missing dependency in workflow `{workflow_name}`: step `{step_id}` depends on unknown step `{dependency}`")]
    MissingDependency {
        workflow_name: String,
        step_id: String,
        dependency: String,
    },

    #[error("expression resolution error: {reason}")]
    ExpressionResolution { reason: String },

    #[error("workflow execution timed out after {timeout_secs}s")]
    ExecutionTimeout { timeout_secs: u64 },

    #[error("workflow execution aborted: {reason}")]
    ExecutionAborted { reason: String },

    #[error("missing required variable: `{var_name}`")]
    MissingVariable { var_name: String },

    #[error("internal workflow error: {reason}")]
    InternalError { reason: String },

    #[error("skill `{skill_name}` not found for step `{step_id}`")]
    SkillNotFound { skill_name: String, step_id: String },

    #[error("executor error during workflow execution: {0}")]
    Executor(#[from] ExecutorError),

    #[error("guardian validation failed during workflow execution: {0}")]
    Guardian(#[from] GuardianError),

    #[error("io error during workflow execution: {0}")]
    Io(#[from] std::io::Error),
}

pub type AppResult<T> = Result<T, AppError>;
pub type ConfigResult<T> = Result<T, ConfigError>;
pub type SkillResult<T> = Result<T, SkillError>;
pub type EmbeddingResult<T> = Result<T, EmbeddingError>;
pub type LlmResult<T> = Result<T, LlmError>;
pub type SlotFillingResult<T> = Result<T, SlotFillingError>;
pub type GuardianResult<T> = Result<T, GuardianError>;
pub type ExecutorResult<T> = Result<T, ExecutorError>;
pub type WorkflowResult<T> = Result<T, WorkflowError>;
