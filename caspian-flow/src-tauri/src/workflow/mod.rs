//! Workflow engine — DAG-based orchestration of skills.
//!
//! ## P17 scope
//!
//! This module is the **core orchestrator** for workflow execution:
//!
//! 1. Parse a workflow (`schema`) and build a DAG from `step.depends_on`.
//! 2. Topologically sort the DAG and detect dependency cycles (`dag`).
//! 3. Execute steps **sequentially** in topological order (`engine`).
//! 4. Resolve `${variables.*}` / `${steps.<id>.output.*}` references via
//!    [`expression`], using an in-memory execution context.
//! 5. Delegate skill execution to the existing [`Executor`][crate::skill::executor::Executor]
//!    and output validation to the existing [`Guardian`][crate::guardian::Guardian]
//!    — the workflow engine does **not** re-implement execution or validation.
//! 6. Persist every step output and run state to `~/.caspian/temp/workflows/<run_id>/`
//!    (`store`) so runs are queryable after completion.
//!
//! ## Scope boundaries (this phase)
//!
//! - **Sequential** execution only. Parallel scheduling is P19.
//! - `condition` / `iterate` / `vars` / `end` are **not** evaluated here (P18).
//! - Only the `abort` error-handling strategy is supported in P17; any other
//!   value returns an explicit `ValidationError` rather than undefined behavior.
//! - Basic per-run file storage only; no reference counting or LRU eviction
//!   (P20 intermediate-result caching).

pub mod cache;
pub mod dag;
pub mod engine;
pub mod expression;
pub mod manifest;
pub mod runner;
pub mod scanner;
pub mod scheduler;
pub mod schema;
pub mod store;

pub use cache::{CacheEntry, CacheStatus, CacheStore};
pub use dag::{compute_topology, TopologyResult};
pub use engine::{StepResult, WorkflowEngine, WorkflowRunResult};
pub use expression::{evaluate_condition, resolve_value, ExpressionContext};
pub use manifest::{
    delete_workflow, list_entries, read_raw, read_workflow, save_draft, save_workflow,
    WorkflowListEntry,
};
pub use schema::{
    ErrorHandlingStrategy, Workflow, WorkflowErrorHandling, WorkflowStep, WorkflowVariable,
    WORKFLOW_SCHEMA_VERSION,
};
pub use scanner::{modified_at, WorkflowScanner, WorkflowSummary};
pub use store::{RunRecord, RunStatus, RunStore, StepRecord};
