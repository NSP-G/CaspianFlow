//! Workflow execution engine — the DAG orchestrator (P17 core).
//!
//! The engine is deliberately thin: it sequences step execution in topological
//! order, resolves `${...}` references against an in-memory context, and
//! persists run state. Skill execution is delegated to
//! [`Executor`][crate::skill::executor::Executor] and output validation to
//! [`Guardian`][crate::guardian::Guardian] — this module does **not**
//! re-implement either.
//!
//! From P19 the engine delegates the actual scheduling to
//! [`scheduler::run_schedule`], which runs independent steps **concurrently**
//! (bounded by `Workflow::parallelism`, default 4) while keeping all shared
//! state mutation single-threaded inside the driver loop.
//!
//! P18 control flow (`if` guards, `vars`, `iterate`, `end`) is honoured inside
//! the scheduler.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::guardian::Guardian;
use crate::skill::executor::Executor;
use crate::skill::registry::SkillRegistry;
use crate::types::{WorkflowError, WorkflowResult};
use crate::workflow::expression::ExpressionContext;
use crate::workflow::schema::Workflow;
use crate::workflow::store::{RunRecord, RunStatus, RunStore, StepRecord};

/// Result of executing a single workflow step.
#[derive(Debug, Clone)]
pub struct StepResult {
    pub step_id: String,
    pub skill: String,
    pub output: Value,
    pub duration_ms: u64,
}

/// Result of executing an entire workflow.
#[derive(Debug, Clone)]
pub struct WorkflowRunResult {
    pub run_id: String,
    pub workflow_name: String,
    /// Final in-memory execution context: `step_id -> output`.
    pub outputs: HashMap<String, Value>,
    /// Per-step results in execution order.
    pub steps: Vec<StepResult>,
    pub duration_ms: u64,
    /// True if the run stopped early because an `end` condition was met (P18).
    pub terminated: bool,
    /// Number of steps skipped because their `if` condition was false (P18).
    pub skipped_steps: usize,
}

/// The workflow engine — orchestrates DAG execution of skills.
///
/// `executor` and `guardian` are held behind `Arc` rather than cloned per task:
/// `Executor` owns a concurrency-limiting pool and `Guardian` accumulates a
/// validation log, so every spawned step must share *one* instance (P19).
pub struct WorkflowEngine {
    registry: Arc<SkillRegistry>,
    store: Arc<RunStore>,
    executor: Arc<Executor>,
    guardian: Arc<Guardian>,
}

impl WorkflowEngine {
    /// Build an engine with explicit collaborators (test-friendly: inject a
    /// custom executor / guardian / store / registry).
    pub fn new(
        registry: Arc<SkillRegistry>,
        store: Arc<RunStore>,
        executor: Executor,
        guardian: Guardian,
    ) -> Self {
        Self {
            registry,
            store,
            executor: Arc::new(executor),
            guardian: Arc::new(guardian),
        }
    }

    /// Build an engine with default executor and guardian.
    pub fn with_defaults(registry: Arc<SkillRegistry>, store: Arc<RunStore>) -> Self {
        Self {
            registry,
            store,
            executor: Arc::new(Executor::with_defaults()),
            guardian: Arc::new(Guardian::with_defaults()),
        }
    }

    /// Execute a workflow to completion.
    ///
    /// `inputs` are the workflow-level inputs (the spec's `${input.field}`,
    /// mapped onto `${variables.<name>}` in this implementation — see the
    /// design report). Returns the run result with every step output.
    ///
    /// # Errors
    ///
    /// Returns a [`WorkflowError`] on:
    /// - unsupported error-handling strategy (non-`abort`),
    /// - cycle / missing dependency (from topology),
    /// - missing required input variable,
    /// - unknown skill (`SkillNotFound`),
    /// - executor or guardian failure (propagated via `?`),
    /// - first step failure (`StepFailed`, run marked `Failed`).
    pub async fn execute(
        &self,
        workflow: &Workflow,
        inputs: &HashMap<String, Value>,
    ) -> WorkflowResult<WorkflowRunResult> {
        crate::workflow::scheduler::run_schedule(
            workflow,
            inputs,
            self.registry.clone(),
            self.store.clone(),
            self.executor.clone(),
            self.guardian.clone(),
        )
        .await
    }
}

/// Record a skipped step (its `if` condition was false or it transitively
/// depended on one) without executing it. A skip is not a failure (P18).
///
/// `pub(crate)` so the parallel [`scheduler`] can reuse it.
pub(crate) fn record_skipped(
    run: &mut RunRecord,
    step_results: &mut Vec<StepResult>,
    step_id: &str,
    skill: &str,
) {
    run.steps.push(StepRecord {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: RunStatus::Skipped,
        duration_ms: 0,
        error: None,
        output_path: PathBuf::new(),
    });
    step_results.push(StepResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        output: Value::Null,
        duration_ms: 0,
    });
}

/// Seed the execution context from workflow variable defaults, provided
/// inputs, and validate required variables are present.
///
/// `pub(crate)` so the parallel [`scheduler`] can reuse it.
pub(crate) fn seed_context(
    workflow: &Workflow,
    inputs: &HashMap<String, Value>,
    ctx: &mut ExpressionContext,
) -> WorkflowResult<()> {
    // Apply defaults from the workflow variable definitions first.
    for var in &workflow.variables {
        if let Some(default) = &var.default {
            ctx.set_variable(&var.name, default.clone());
        }
    }

    // Override with provided inputs.
    for (name, value) in inputs {
        ctx.set_variable(name, value.clone());
    }

    // Required variables must be present after defaults + inputs.
    for var in &workflow.variables {
        if var.required && !ctx.variables.contains_key(&var.name) {
            return Err(WorkflowError::MissingVariable {
                var_name: var.name.clone(),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::{Skill, SkillRuntime, SkillRuntimeType};
    use crate::workflow::store::RunStore;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// Register a Shell-runtime test skill whose `run.sh` is `script`.
    fn make_shell_skill(registry: &SkillRegistry, name: &str, base: &Path, script: &str) -> Skill {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let script_path = dir.join("run.sh");
        std::fs::write(&script_path, script).unwrap();
        // Ensure executable so the shell adapter can run it directly.
        let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755));
        let skill = Skill {
            schema_version: "1.0".into(),
            name: name.into(),
            display_name: name.into(),
            version: "1.0.0".into(),
            description: format!("test skill {name}"),
            category: "test".into(),
            trigger_phrases: vec!["test".into()],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Shell,
                entry: "run.sh".into(),
                timeout: 10,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            // P32: the sandbox refuses Shell skills that don't declare shell access.
            permissions: crate::skill::schema::SkillPermissions {
                shell: true,
                ..Default::default()
            },
            tags: vec![],
            author: "test".into(),
            license: "MIT".into(),
            enabled: true,
            path: dir,
            mcp: None,
        };
        registry.register(skill.clone());
        skill
    }

    /// `(tempdir, registry, store, engine)` — all owned by the caller.
    fn fixture() -> (
        tempfile::TempDir,
        Arc<SkillRegistry>,
        Arc<RunStore>,
        WorkflowEngine,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(SkillRegistry::new());
        let store = Arc::new(RunStore::new(dir.path()));
        let engine = WorkflowEngine::with_defaults(registry.clone(), store.clone());
        (dir, registry, store, engine)
    }

    #[tokio::test]
    async fn test_linear_dag_3_nodes() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "double", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "format", base, "#!/bin/sh\ncat\n");

        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "double"
    skill: "double"
    input:
      base: "${steps.gen.output.n}"
    depends_on: ["gen"]
  - id: "format"
    skill: "format"
    input:
      value: "${steps.double.output.base}"
    depends_on: ["double"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();

        assert_eq!(result.steps.len(), 3);
        assert_eq!(result.steps[0].step_id, "gen");
        assert_eq!(result.steps[2].step_id, "format");
        assert_eq!(result.steps[2].output["value"], serde_json::json!(7));
    }

    #[tokio::test]
    async fn test_fork_join_5_nodes() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(
            &registry,
            "source",
            base,
            "#!/bin/sh\necho '{\"seed\": 100}'\n",
        );
        make_shell_skill(&registry, "left", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "right", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "merge", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "report", base, "#!/bin/sh\ncat\n");

        let yaml = r#"
name: "wf"
steps:
  - id: "source"
    skill: "source"
    input: {}
  - id: "left"
    skill: "left"
    input:
      x: "${steps.source.output.seed}"
    depends_on: ["source"]
  - id: "right"
    skill: "right"
    input:
      x: "${steps.source.output.seed}"
    depends_on: ["source"]
  - id: "merge"
    skill: "merge"
    input:
      a: "${steps.left.output.x}"
      b: "${steps.right.output.x}"
    depends_on: ["left", "right"]
  - id: "report"
    skill: "report"
    input:
      sum: "${steps.merge.output.a}"
      prod: "${steps.merge.output.b}"
    depends_on: ["merge"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();

        assert_eq!(result.steps.len(), 5);
        assert_eq!(result.steps[4].output["sum"], serde_json::json!(100));
        assert_eq!(result.steps[4].output["prod"], serde_json::json!(100));
    }

    #[tokio::test]
    async fn test_workflow_input_passthrough() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "greet", base, "#!/bin/sh\ncat\n");

        let yaml = r#"
name: "wf"
variables:
  - name: "name"
    type: "string"
    required: true
steps:
  - id: "greet"
    skill: "greet"
    input:
      name: "${variables.name}"
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();

        let mut inputs = HashMap::new();
        inputs.insert("name".to_string(), serde_json::json!("Keel"));
        let result = engine.execute(&wf, &inputs).await.unwrap();

        assert_eq!(result.steps[0].output["name"], serde_json::json!("Keel"));
    }

    #[tokio::test]
    async fn test_missing_required_input_variable() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "greet", base, "#!/bin/sh\ncat\n");

        let yaml = r#"
name: "wf"
variables:
  - name: "name"
    type: "string"
    required: true
steps:
  - id: "greet"
    skill: "greet"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let res = engine.execute(&wf, &HashMap::new()).await;
        assert!(matches!(res, Err(WorkflowError::MissingVariable { .. })));
    }

    #[tokio::test]
    async fn test_step_reference_resolution_direct_form() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "use", base, "#!/bin/sh\ncat\n");

        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "use"
    skill: "use"
    input:
      val: "${steps.gen.n}"
    depends_on: ["gen"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        // `output` view alias and direct form are equivalent.
        assert_eq!(result.steps[1].output["val"], serde_json::json!(7));
    }

    #[tokio::test]
    async fn test_middle_step_failure_aborts_and_records_run() {
        let (_dir, registry, store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "a", base, "#!/bin/sh\necho '{\"ok\": true}'\n");
        make_shell_skill(&registry, "b", base, "#!/bin/sh\necho 'boom' >&2\nexit 1\n");
        make_shell_skill(&registry, "c", base, "#!/bin/sh\ncat\n");

        let yaml = r#"
name: "wf"
steps:
  - id: "a"
    skill: "a"
    input: {}
  - id: "b"
    skill: "b"
    input: {}
    depends_on: ["a"]
  - id: "c"
    skill: "c"
    input: {}
    depends_on: ["b"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let res = engine.execute(&wf, &HashMap::new()).await;
        assert!(matches!(
            res,
            Err(WorkflowError::StepFailed { step_id, .. }) if step_id == "b"
        ));

        // The run must be persisted as Failed, and `c` must not have run.
        let runs = store.list_runs().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Failed);
        assert!(runs[0]
            .steps
            .iter()
            .any(|s| s.step_id == "b" && s.status == RunStatus::Failed));
        assert!(!runs[0].steps.iter().any(|s| s.step_id == "c"));
    }

    #[tokio::test]
    async fn test_non_abort_error_handling_rejected() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "noop", base, "#!/bin/sh\necho '{}'\n");

        let yaml = r#"
name: "wf"
error_handling:
  on_step_failure: "continue"
steps:
  - id: "s1"
    skill: "noop"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let res = engine.execute(&wf, &HashMap::new()).await;
        assert!(matches!(res, Err(WorkflowError::ValidationError { .. })));
    }

    #[tokio::test]
    async fn test_unknown_skill_reports_step_failed() {
        let (_dir, _registry, _store, engine) = fixture();
        let yaml = r#"
name: "wf"
steps:
  - id: "x"
    skill: "does_not_exist"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let res = engine.execute(&wf, &HashMap::new()).await;
        assert!(matches!(
            res,
            Err(WorkflowError::StepFailed { step_id, .. }) if step_id == "x"
        ));
    }

    #[test]
    fn test_yaml_parse_error_has_reason() {
        let res = Workflow::from_yaml("steps: [ this is : not : valid : yaml");
        assert!(res.is_err());
        match res {
            Err(WorkflowError::ParseError { path, reason }) => {
                assert!(!reason.is_empty());
                assert!(path.contains("(inline)"));
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // P18 — control flow
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_if_false_skips_step() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "use", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "use"
    skill: "use"
    input: {}
    depends_on: ["gen"]
    condition: "${steps.gen.output.n} > 100"
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[1].step_id, "use");
        assert_eq!(result.steps[1].output, serde_json::Value::Null);
        assert_eq!(result.skipped_steps, 1);
    }

    #[tokio::test]
    async fn test_if_true_runs_step() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "use", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "use"
    skill: "use"
    input: {}
    depends_on: ["gen"]
    condition: "${steps.gen.output.n} > 0"
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert_eq!(result.skipped_steps, 0);
        assert_eq!(result.steps[1].output, serde_json::json!({}));
    }

    #[tokio::test]
    async fn test_skipped_dependents_skipped() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "mid", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "tail", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "mid"
    skill: "mid"
    input: {}
    depends_on: ["gen"]
    condition: "${steps.gen.output.n} > 100"
  - id: "tail"
    skill: "tail"
    input: {}
    depends_on: ["mid"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        // `mid` skipped by its false `if`; `tail` skipped transitively.
        assert_eq!(result.skipped_steps, 2);
        assert_eq!(result.steps[1].output, serde_json::Value::Null);
        assert_eq!(result.steps[2].output, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_vars_assignment_propagates() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 5}'\n");
        make_shell_skill(&registry, "assign", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "consumer", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "assign"
    skill: "assign"
    input: {}
    depends_on: ["gen"]
    vars:
      doubled: "${steps.gen.output.n}"
  - id: "consumer"
    skill: "consumer"
    input:
      v: "${variables.doubled}"
    depends_on: ["assign"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert_eq!(result.skipped_steps, 0);
        // `vars.doubled` resolved to the upstream number and reached the consumer.
        assert_eq!(result.steps[2].output["v"], serde_json::json!(5));
    }

    #[tokio::test]
    async fn test_iterate_collects_array() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(
            &registry,
            "gen",
            base,
            "#!/bin/sh\necho '{\"items\": [1, 2, 3]}'\n",
        );
        make_shell_skill(&registry, "each", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "each"
    skill: "each"
    input:
      x: "${item}"
    depends_on: ["gen"]
    iterate: "${steps.gen.output.items}"
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert_eq!(result.skipped_steps, 0);
        let out = &result.steps[1].output;
        assert!(out.is_array());
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["x"], serde_json::json!(1));
        assert_eq!(arr[2]["x"], serde_json::json!(3));
    }

    #[tokio::test]
    async fn test_iterate_binds_as_var() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(
            &registry,
            "gen",
            base,
            "#!/bin/sh\necho '{\"items\": [\"a\", \"b\"]}'\n",
        );
        make_shell_skill(&registry, "each", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "each"
    skill: "each"
    input:
      letter: "${variables.ch}"
    depends_on: ["gen"]
    iterate: "${steps.gen.output.items}"
    as_var: "ch"
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        let out = &result.steps[1].output;
        assert!(out.is_array());
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["letter"], serde_json::json!("a"));
        assert_eq!(arr[1]["letter"], serde_json::json!("b"));
    }

    #[tokio::test]
    async fn test_end_terminates_early() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(
            &registry,
            "first",
            base,
            "#!/bin/sh\necho '{\"done\": true}'\n",
        );
        make_shell_skill(&registry, "second", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "third", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
end:
  if: "${steps.first.output.done} == true"
steps:
  - id: "first"
    skill: "first"
    input: {}
  - id: "second"
    skill: "second"
    input: {}
    depends_on: ["first"]
  - id: "third"
    skill: "third"
    input: {}
    depends_on: ["second"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert!(result.terminated);
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].step_id, "first");
    }

    #[tokio::test]
    async fn test_end_not_hit_runs_to_completion() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(
            &registry,
            "first",
            base,
            "#!/bin/sh\necho '{\"done\": false}'\n",
        );
        make_shell_skill(&registry, "second", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
end:
  if: "${steps.first.output.done} == true"
steps:
  - id: "first"
    skill: "first"
    input: {}
  - id: "second"
    skill: "second"
    input: {}
    depends_on: ["first"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert!(!result.terminated);
        assert_eq!(result.steps.len(), 2);
    }

    // -----------------------------------------------------------------------
    // P19: parallel scheduler acceptance tests
    // -----------------------------------------------------------------------

    /// Criterion 1 — independent steps run *truly* concurrently: total wall
    /// time is well under the serial sum. Four 0.2s steps would take ~0.8s
    /// serial; with the default parallelism (4) they overlap to ~0.2s.
    #[tokio::test]
    async fn test_parallel_steps_run_concurrently() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        for name in ["a", "b", "c", "d"] {
            make_shell_skill(
                &registry,
                name,
                base,
                "#!/bin/sh\nsleep 0.2\necho '{\"x\": 1}'\n",
            );
        }
        let yaml = r#"
name: "wf"
steps:
  - id: "a"
    skill: "a"
    input: {}
  - id: "b"
    skill: "b"
    input: {}
  - id: "c"
    skill: "c"
    input: {}
  - id: "d"
    skill: "d"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let start = std::time::Instant::now();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        let elapsed = start.elapsed();
        // Concurrent (~0.2s) must beat serial (0.8s) with margin for spawn cost.
        assert!(
            elapsed.as_millis() < 600,
            "expected concurrent < 600ms, got {}ms",
            elapsed.as_millis()
        );
        assert_eq!(result.steps.len(), 4);
    }

    /// Criterion 2 — dependency ordering is preserved under concurrency: a
    /// dependent step only runs after its upstream completed and sees its output.
    #[tokio::test]
    async fn test_dependency_ordering_under_parallelism() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "use", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
parallelism: 4
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "use"
    skill: "use"
    input:
      v: "${steps.gen.output.n}"
    depends_on: ["gen"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        // `use` must carry `gen`'s output — proves it ran strictly after `gen`.
        assert_eq!(result.steps.len(), 2);
        let use_out = result.steps.iter().find(|s| s.step_id == "use").unwrap();
        assert_eq!(use_out.output["v"], serde_json::json!(7));
    }

    /// Criterion 3 — fork/join: two independent middle steps run concurrently
    /// and the join step waits for both, seeing each branch's output.
    #[tokio::test]
    async fn test_fork_join_concurrent() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(
            &registry,
            "source",
            base,
            "#!/bin/sh\necho '{\"seed\": 100}'\n",
        );
        make_shell_skill(
            &registry,
            "left",
            base,
            "#!/bin/sh\nsleep 0.2\necho '{\"side\": \"L\"}'\n",
        );
        make_shell_skill(
            &registry,
            "right",
            base,
            "#!/bin/sh\nsleep 0.2\necho '{\"side\": \"R\"}'\n",
        );
        make_shell_skill(&registry, "merge", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
parallelism: 4
steps:
  - id: "source"
    skill: "source"
    input: {}
  - id: "left"
    skill: "left"
    input: {}
    depends_on: ["source"]
  - id: "right"
    skill: "right"
    input: {}
    depends_on: ["source"]
  - id: "merge"
    skill: "merge"
    input:
      l: "${steps.left.output.side}"
      r: "${steps.right.output.side}"
    depends_on: ["left", "right"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let start = std::time::Instant::now();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        let elapsed = start.elapsed();
        // `left`+`right` overlap (0.2s), not serial (0.4s).
        assert!(
            elapsed.as_millis() < 600,
            "expected fork/join < 600ms, got {}ms",
            elapsed.as_millis()
        );
        let merge = result.steps.iter().find(|s| s.step_id == "merge").unwrap();
        assert_eq!(merge.output["l"], serde_json::json!("L"));
        assert_eq!(merge.output["r"], serde_json::json!("R"));
    }

    /// Criterion 4 — the concurrency gate bounds in-flight work: with
    /// `parallelism: 2` and four 0.2s steps we get two ~0.2s batches (~0.4s),
    /// not all-at-once (~0.2s) and not serial (~0.8s).
    #[tokio::test]
    async fn test_parallelism_gate_limits_in_flight() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        for name in ["a", "b", "c", "d"] {
            make_shell_skill(
                &registry,
                name,
                base,
                "#!/bin/sh\nsleep 0.2\necho '{\"x\": 1}'\n",
            );
        }
        let yaml = r#"
name: "wf"
parallelism: 2
steps:
  - id: "a"
    skill: "a"
    input: {}
  - id: "b"
    skill: "b"
    input: {}
  - id: "c"
    skill: "c"
    input: {}
  - id: "d"
    skill: "d"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let start = std::time::Instant::now();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        let elapsed = start.elapsed();
        // Two batches of 0.2s: bounded below (not all-at-once) and above
        // (not serial). Generous margins for spawn overhead.
        assert!(
            elapsed.as_millis() > 300,
            "expected gated > 300ms (not all-at-once), got {}ms",
            elapsed.as_millis()
        );
        assert!(
            elapsed.as_millis() < 700,
            "expected gated < 700ms (not serial), got {}ms",
            elapsed.as_millis()
        );
        assert_eq!(result.steps.len(), 4);
    }

    /// Criterion 5 — `parallelism: 1` degrades to strict sequential execution.
    /// Three 0.2s steps take ~0.6s, far above the ~0.2s a concurrent run would.
    #[tokio::test]
    async fn test_parallelism_one_is_sequential() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        for name in ["a", "b", "c"] {
            make_shell_skill(
                &registry,
                name,
                base,
                "#!/bin/sh\nsleep 0.2\necho '{\"x\": 1}'\n",
            );
        }
        let yaml = r#"
name: "wf"
parallelism: 1
steps:
  - id: "a"
    skill: "a"
    input: {}
  - id: "b"
    skill: "b"
    input: {}
  - id: "c"
    skill: "c"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let start = std::time::Instant::now();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        let elapsed = start.elapsed();
        // Sequential: three 0.2s steps must exceed one 0.2s step's budget.
        assert!(
            elapsed.as_millis() > 500,
            "expected sequential > 500ms, got {}ms",
            elapsed.as_millis()
        );
        assert_eq!(result.steps.len(), 3);
    }

    /// Criterion 6 — `if` skip propagation holds under concurrent scheduling:
    /// a false `condition` skips a step and transitively its dependents, even
    /// with a parallel branch running alongside.
    #[tokio::test]
    async fn test_if_skip_propagates_under_parallelism() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "mid", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "tail", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "par", base, "#!/bin/sh\necho '{\"p\": 1}'\n");
        let yaml = r#"
name: "wf"
parallelism: 2
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "mid"
    skill: "mid"
    input: {}
    depends_on: ["gen"]
    condition: "${steps.gen.output.n} > 100"
  - id: "tail"
    skill: "tail"
    input: {}
    depends_on: ["mid"]
  - id: "par"
    skill: "par"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        // `mid` skipped by its false condition; `tail` transitively skipped.
        assert_eq!(result.skipped_steps, 2);
        let mid = result.steps.iter().find(|s| s.step_id == "mid").unwrap();
        let tail = result.steps.iter().find(|s| s.step_id == "tail").unwrap();
        assert_eq!(mid.output, serde_json::Value::Null);
        assert_eq!(tail.output, serde_json::Value::Null);
        // The independent parallel branch still ran.
        let par = result.steps.iter().find(|s| s.step_id == "par").unwrap();
        assert_eq!(par.output["p"], serde_json::json!(1));
    }

    /// Criterion 7 — failure Abort: a failing step stops *new* scheduling, but
    /// an already in-flight step is allowed to finish (we do not force-abort
    /// external subprocesses). Downstream of the failed step is never run.
    #[tokio::test]
    async fn test_failure_aborts_no_new_scheduling() {
        let (_dir, registry, store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "ok", base, "#!/bin/sh\necho '{\"v\": 1}'\n");
        // Fails after a short sleep so an in-flight sibling can start first.
        make_shell_skill(
            &registry,
            "boom",
            base,
            "#!/bin/sh\nsleep 0.1\necho boom >&2\nexit 1\n",
        );
        make_shell_skill(
            &registry,
            "busy",
            base,
            "#!/bin/sh\nsleep 0.5\necho '{\"v\": 2}'\n",
        );
        make_shell_skill(&registry, "orphan", base, "#!/bin/sh\necho '{\"v\": 3}'\n");
        let yaml = r#"
name: "wf"
parallelism: 4
steps:
  - id: "ok"
    skill: "ok"
    input: {}
  - id: "boom"
    skill: "boom"
    input: {}
  - id: "busy"
    skill: "busy"
    input: {}
    depends_on: ["ok"]
  - id: "orphan"
    skill: "orphan"
    input: {}
    depends_on: ["boom"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let res = engine.execute(&wf, &HashMap::new()).await;
        assert!(matches!(
            res,
            Err(WorkflowError::StepFailed { step_id, .. }) if step_id == "boom"
        ));

        // Inspect the persisted run: `ok` and `busy` (in-flight) completed,
        // `boom` failed, `orphan` (downstream of the failure) never ran.
        let runs = store.list_runs().unwrap();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.status, RunStatus::Failed);
        let ids: Vec<&str> = run.steps.iter().map(|s| s.step_id.as_str()).collect();
        assert!(ids.contains(&"ok"));
        assert!(ids.contains(&"boom"));
        assert!(ids.contains(&"busy"));
        assert!(!ids.contains(&"orphan"));
        let boom_rec = run.steps.iter().find(|s| s.step_id == "boom").unwrap();
        assert_eq!(boom_rec.status, RunStatus::Failed);
        let busy_rec = run.steps.iter().find(|s| s.step_id == "busy").unwrap();
        assert_eq!(busy_rec.status, RunStatus::Completed);
    }

    /// Criterion 8 — `end` early-termination stops downstream scheduling even
    /// under concurrency: a fork downstream of the terminating step never runs.
    #[tokio::test]
    async fn test_end_terminates_early_under_parallelism() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(
            &registry,
            "first",
            base,
            "#!/bin/sh\necho '{\"done\": true}'\n",
        );
        make_shell_skill(&registry, "second", base, "#!/bin/sh\ncat\n");
        make_shell_skill(&registry, "third", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
parallelism: 4
end:
  if: "${steps.first.output.done} == true"
steps:
  - id: "first"
    skill: "first"
    input: {}
  - id: "second"
    skill: "second"
    input: {}
    depends_on: ["first"]
  - id: "third"
    skill: "third"
    input: {}
    depends_on: ["first"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert!(result.terminated);
        // Only `first` ran; the fork downstream of the `end` was not scheduled.
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].step_id, "first");
    }

    /// Criterion 9 — every step output is persisted to `<run_id>/steps/<id>.json`.
    #[tokio::test]
    async fn test_step_output_persisted_to_disk() {
        let (_dir, registry, store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "gen", base, "#!/bin/sh\necho '{\"n\": 7}'\n");
        make_shell_skill(&registry, "use", base, "#!/bin/sh\ncat\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "gen"
    skill: "gen"
    input: {}
  - id: "use"
    skill: "use"
    input:
      v: "${steps.gen.output.n}"
    depends_on: ["gen"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let result = engine.execute(&wf, &HashMap::new()).await.unwrap();

        let runs = store.list_runs().unwrap();
        assert_eq!(runs.len(), 1);
        for rec in &runs[0].steps {
            assert!(
                rec.output_path.exists(),
                "missing persisted output for step {}",
                rec.step_id
            );
            let raw = std::fs::read_to_string(&rec.output_path).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
            // The persisted file must match the in-memory result output.
            let live = result
                .steps
                .iter()
                .find(|s| s.step_id == rec.step_id)
                .unwrap();
            assert_eq!(parsed, live.output);
        }
    }

    /// Regression — a dependency cycle is still rejected (topology validation
    /// runs before any scheduling, so the parallel path cannot hide a cycle).
    #[tokio::test]
    async fn test_cycle_still_rejected() {
        let (_dir, registry, _store, engine) = fixture();
        let base = _dir.path();
        make_shell_skill(&registry, "a", base, "#!/bin/sh\necho '{}'\n");
        make_shell_skill(&registry, "b", base, "#!/bin/sh\necho '{}'\n");
        let yaml = r#"
name: "wf"
steps:
  - id: "a"
    skill: "a"
    input: {}
    depends_on: ["b"]
  - id: "b"
    skill: "b"
    input: {}
    depends_on: ["a"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let res = engine.execute(&wf, &HashMap::new()).await;
        assert!(matches!(
            res,
            Err(WorkflowError::CycleDetected { .. }) | Err(WorkflowError::MissingDependency { .. })
        ));
    }

    // ---- P20: intermediate-result cache (acceptance items) -------------------

    /// Acceptance #1: identical inputs -> second run skips execution, serves cache.
    /// The shell skill appends to a marker file on every *real* execution; a cache
    /// hit must NOT append. This is the honest signal that execution was skipped
    /// (not a duration check, which a fast real step could also satisfy).
    #[tokio::test]
    async fn test_cache_reuses_output_on_identical_inputs() {
        let (dir, registry, _store, engine) = fixture();
        let base = dir.path();
        let marker = base.join("exec_count");
        let script = format!(
            "#!/bin/sh\nprintf 'x' >> {m}\necho '{{\"value\": 42}}'\n",
            m = marker.display()
        );
        make_shell_skill(&registry, "compute", base, &script);
        let yaml = r#"
name: "wf_cache"
schema_version: "1.0"
steps:
  - id: "s1"
    skill: "compute"
    input: {}
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();

        let r1 = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert_eq!(r1.steps[0].output, serde_json::json!({"value": 42}));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");

        // Second run with identical inputs -> cache hit, no re-execution.
        let r2 = engine.execute(&wf, &HashMap::new()).await.unwrap();
        assert_eq!(r2.steps[0].output, serde_json::json!({"value": 42}));
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "x",
            "step must not re-execute on a cache hit"
        );
    }

    /// Acceptance #2: an upstream output change forces the downstream to recompute.
    /// A echoes its input; B depends on A. Changing the input changes A's output,
    /// which changes B's `upstream_output_hash` -> B's key differs -> B re-executes.
    #[tokio::test]
    async fn test_cache_recomputes_when_upstream_changes() {
        let (dir, registry, _store, engine) = fixture();
        let base = dir.path();
        let a_marker = base.join("a_count");
        let b_marker = base.join("b_count");
        let a_script = format!(
            "#!/bin/sh\nprintf 'a' >> {m}\ncat\n",
            m = a_marker.display()
        );
        let b_script = format!(
            "#!/bin/sh\nprintf 'b' >> {m}\ncat\n",
            m = b_marker.display()
        );
        make_shell_skill(&registry, "a", base, &a_script);
        make_shell_skill(&registry, "b", base, &b_script);
        let yaml = r#"
name: "wf_dep"
schema_version: "1.0"
steps:
  - id: "a"
    skill: "a"
    input:
      x: "${variables.x}"
  - id: "b"
    skill: "b"
    input:
      from_a: "${steps.a.output.x}"
    depends_on: ["a"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let inputs = |v: i64| HashMap::from([("x".to_string(), serde_json::json!(v))]);

        // run1: x=1, both execute.
        let _r1 = engine.execute(&wf, &inputs(1)).await.unwrap();
        assert_eq!(std::fs::read_to_string(&a_marker).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&b_marker).unwrap(), "b");

        // run2: x=1 again, both cached.
        let _r2 = engine.execute(&wf, &inputs(1)).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&a_marker).unwrap(),
            "a",
            "A should be cached"
        );
        assert_eq!(
            std::fs::read_to_string(&b_marker).unwrap(),
            "b",
            "B should be cached"
        );

        // run3: x=2 -> A re-executes (input changed) and B re-executes (upstream
        // output changed). Both markers gain a second append.
        let _r3 = engine.execute(&wf, &inputs(2)).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&a_marker).unwrap(),
            "aa",
            "A must recompute when its input changes"
        );
        assert_eq!(
            std::fs::read_to_string(&b_marker).unwrap(),
            "bb",
            "B must recompute when its upstream output changes"
        );
    }

    /// Acceptance #4 (end-to-end): a `schema_version` bump must invalidate the
    /// prior cache so the step re-executes. The explicit stale-marking is covered
    /// by the `cache.rs` unit test; this confirms the observable behavior.
    #[tokio::test]
    async fn test_cache_recomputes_on_schema_version_upgrade() {
        let (dir, registry, _store, engine) = fixture();
        let base = dir.path();
        let marker = base.join("exec_count");
        let script = format!(
            "#!/bin/sh\nprintf 'r' >> {m}\necho '{{\"value\": 1}}'\n",
            m = marker.display()
        );
        make_shell_skill(&registry, "compute", base, &script);
        let mk = |ver: &str| {
            Workflow::from_yaml(&format!(
                r#"
name: "wf_ver"
schema_version: "{ver}"
steps:
  - id: "s1"
    skill: "compute"
    input: {{}}
"#
            ))
            .unwrap()
        };

        let _r1 = engine.execute(&mk("1.0"), &HashMap::new()).await.unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "r");

        // Same logic, same inputs, schema bumped -> cache must be invalidated.
        let _r2 = engine.execute(&mk("2.0"), &HashMap::new()).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "rr",
            "schema bump must invalidate the prior cache"
        );

        // Running 2.0 again now hits the (new-version) cache.
        let _r3 = engine.execute(&mk("2.0"), &HashMap::new()).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "rr",
            "2.0 should be cached after its first run"
        );
    }
}
