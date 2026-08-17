//! Parallel workflow scheduler (P19).
//!
//! Replaces the sequential `for step in topo.order` loop (P17) with a
//! ready-queue + dependency-count + concurrency-gate model:
//!
//! ```text
//! init:  all steps with indegree 0  -> ready queue
//! loop:  pop a ready step, spawn a tokio task to execute it
//!        on completion: decrement each dependent's remaining-indegree
//!        dependent's indegree hits 0 -> push to ready queue
//!        repeat until all steps complete, or one fails / `end` fires
//! ```
//!
//! Shared mutable state (`ctx`, `outputs`, `run`, `skipped`, ...) is owned by a
//! single `Mutex` inside the driver loop. Spawned tasks perform only **pure
//! execution** (no shared mutation); they report `StepOutcome`s back over an
//! `mpsc` channel. All state mutation therefore stays single-threaded in the
//! driver loop — no data races. Concurrency is bounded by the in-flight task
//! counter (the spec's "parallelism", default 4; `parallelism = 1` degrades to
//! sequential). `tokio` is already `features = ["full"]`, so no new dependency.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde_json::Value;
use tokio::sync::mpsc;

use crate::guardian::Guardian;
use crate::skill::executor::Executor;
use crate::skill::registry::SkillRegistry;
use crate::types::{WorkflowError, WorkflowResult};
use crate::workflow::cache::{
    cache_key_of, make_valid_entry, resolved_inputs_hash, upstream_output_hash, CacheKind,
    CacheStore,
};
use crate::workflow::dag::compute_topology;
use crate::workflow::engine::{record_skipped, seed_context, StepResult, WorkflowRunResult};
use crate::workflow::expression::{evaluate_condition, resolve_value, ExpressionContext};
use crate::workflow::schema::{ErrorHandlingStrategy, Workflow, WorkflowStep};
use crate::workflow::store::{RunRecord, RunStatus, RunStore, StepRecord};

/// Shared, driver-loop-owned state. All mutation happens inside the single
/// driver thread (under the `Mutex`); tasks never mutate this directly.
struct Shared {
    ctx: ExpressionContext,
    outputs: HashMap<String, Value>,
    run: RunRecord,
    step_results: Vec<StepResult>,
    skipped: HashSet<String>,
    remaining: HashMap<String, usize>,
    failed: Option<(String, String)>,
    terminated: bool,
    skipped_count: usize,
    /// P20: cache key computed at submit time, used to write the entry after a
    /// real (non-cached) completion.
    cache_keys: HashMap<String, String>,
    /// P20: step ids whose outcome was served from cache (not executed) — so
    /// `process_outcome` does not write a redundant/fresh cache entry for them.
    cache_served: HashSet<String>,
}

/// Outcome reported by a spawned step task.
enum StepOutcome {
    Completed {
        step_id: String,
        output: Value,
        duration_ms: u64,
    },
    Failed {
        step_id: String,
        reason: String,
        duration_ms: u64,
    },
}

/// Execute a workflow to completion using the parallel scheduler.
///
/// Delegated to by [`WorkflowEngine::execute`][crate::workflow::engine::WorkflowEngine::execute].
/// See the module docs for the concurrency model.
pub(crate) async fn run_schedule(
    workflow: &Workflow,
    inputs: &HashMap<String, Value>,
    registry: Arc<SkillRegistry>,
    store: Arc<RunStore>,
    executor: Arc<Executor>,
    guardian: Arc<Guardian>,
) -> WorkflowResult<WorkflowRunResult> {
    let start = Instant::now();

    // 1. Only the `abort` error-handling strategy is supported (P17/P18/P19).
    if workflow.error_handling.on_step_failure != ErrorHandlingStrategy::Abort {
        return Err(WorkflowError::ValidationError {
            workflow_name: workflow.name.clone(),
            errors: format!(
                "error handling strategy `{}` is not supported (only `abort` is supported)",
                workflow.error_handling.on_step_failure
            ),
        });
    }

    // 2. Topological order (also validates acyclic + dependencies present).
    let topo = compute_topology(workflow)?;

    // 3. Seed the execution context from variable defaults + provided inputs.
    let mut ctx = ExpressionContext::new();
    seed_context(workflow, inputs, &mut ctx)?;

    // 4. Persisted run record.
    let run = store.create_run(&workflow.name)?;

    // Build dependents + remaining-indegree maps.
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    let mut remaining: HashMap<String, usize> = HashMap::new();
    for s in &workflow.steps {
        remaining.insert(s.id.clone(), s.depends_on.len());
        for dep in &s.depends_on {
            dependents
                .entry(dep.clone())
                .or_default()
                .push(s.id.clone());
        }
    }

    let shared = Arc::new(Mutex::new(Shared {
        ctx,
        outputs: HashMap::new(),
        run,
        step_results: Vec::with_capacity(topo.order.len()),
        skipped: HashSet::new(),
        remaining,
        failed: None,
        terminated: false,
        skipped_count: 0,
        cache_keys: HashMap::new(),
        cache_served: HashSet::new(),
    }));

    // P20: workflow-level cache, persisted across runs. Invalidate by version
    // (doc trigger #1) before this run starts — if the index was produced under
    // a different `schema_version`, every entry is marked `stale`.
    let cache = CacheStore::new(store.cache_root(), &workflow.name);
    let _ = cache.invalidate_if_version_changed(&workflow.schema_version);

    // Concurrency gate: spec default 4; `parallelism = 1` degrades to sequential.
    let parallelism = workflow.parallelism.unwrap_or(4).max(1);
    let (tx, mut rx) = mpsc::channel::<StepOutcome>(topo.order.len().max(1));

    let mut ready: VecDeque<String> = topo
        .order
        .iter()
        .filter(|id| shared.lock().unwrap().remaining[id.as_str()] == 0)
        .cloned()
        .collect();

    let mut in_flight = 0usize;
    let mut stopped = false;

    loop {
        // Submit as many ready steps as the concurrency gate allows.
        while !stopped && in_flight < parallelism && !ready.is_empty() {
            let sid = ready.pop_front().unwrap();

            // Skip decision (P18 condition guard + upstream-skip propagation):
            // a step must not run if (a) an upstream was skipped, or (b) its own
            // `condition` evaluates to false. Both are evaluated against the
            // shared ctx *before* submission, which is sound because a step only
            // enters `ready` after its upstreams wrote the ctx (fork ⑤).
            let skip = {
                let g = shared.lock().unwrap();
                let step = workflow
                    .get_step(&sid)
                    .expect("topological order holds valid ids");
                let upstream = step.depends_on.iter().any(|d| g.skipped.contains(d));
                let cond_false = match &step.condition {
                    Some(c) if !upstream => !evaluate_condition(c, &g.ctx)?,
                    _ => false,
                };
                upstream || cond_false
            };
            if skip {
                {
                    let mut g = shared.lock().unwrap();
                    mark_and_propagate_skip(&mut g, workflow, &sid, &dependents, &store)?;
                }
                continue;
            }

            let step = workflow.get_step(&sid).expect("valid id").clone();

            // Fork ⑤: resolve inputs in the driver loop *before* submitting, so
            // no lock is ever held across an `.await` inside the task (which
            // would make the spawned future non-`Send`). Sound because a step
            // only becomes ready after every upstream wrote its output to ctx.
            // `iterate` steps resolve per element inside the task instead.
            let prepared: Option<Value> = if step.iterate.is_some() {
                None
            } else {
                let g = shared.lock().unwrap();
                match resolve_value(&step.input, &g.ctx) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        drop(g);
                        let mut g = shared.lock().unwrap();
                        finish_failed(&mut g, &store, &sid, &e.to_string(), 0, &step.skill)?;
                        stopped = true;
                        break;
                    }
                }
            };
            let local_ctx: Option<ExpressionContext> = if step.iterate.is_some() {
                Some(shared.lock().unwrap().ctx.clone())
            } else {
                None
            };

            // P20: compute the deterministic cache key from the five runtime
            // fields, then check for a hit. The key must be identical to what
            // `process_outcome` will recompute after a real execution, so a hit
            // and a fresh write always agree.
            let cache_key = {
                let g = shared.lock().unwrap();
                let upstream_hash = upstream_output_hash(&step.depends_on, &g.outputs);
                let resolved_hash = if let Some(iter_ref) = &step.iterate {
                    let coll = resolve_value(&Value::String(iter_ref.clone()), &g.ctx)
                        .unwrap_or(Value::Null);
                    resolved_inputs_hash(None, Some(&coll), &step.input.to_string())
                } else {
                    let resolved = prepared
                        .as_ref()
                        .expect("non-iterate steps carry resolved input");
                    resolved_inputs_hash(Some(resolved), None, "")
                };
                cache_key_of(
                    &sid,
                    CacheKind::StepOutput.as_str(),
                    &resolved_hash,
                    &upstream_hash,
                    &workflow.schema_version,
                )
            };

            // Cache hit: serve the stored output WITHOUT spawning a task.
            // Synthesize a `Completed` outcome and let `process_outcome` apply it
            // through the single write path; mark `cache_served` so it does not
            // write a redundant entry. `in_flight` is still incremented so the
            // recv handler balances it.
            if let Some(hit) = cache.get(&cache_key) {
                {
                    let mut g = shared.lock().unwrap();
                    g.cache_served.insert(sid.clone());
                }
                in_flight += 1;
                let _ = tx
                    .send(StepOutcome::Completed {
                        step_id: sid.clone(),
                        output: hit.output,
                        duration_ms: 0,
                    })
                    .await;
                continue;
            }

            // Cache miss: remember the key so `process_outcome` writes the entry
            // after this step's real execution completes.
            {
                let mut g = shared.lock().unwrap();
                g.cache_keys.insert(sid.clone(), cache_key);
            }

            in_flight += 1;
            let executor = executor.clone();
            let guardian = guardian.clone();
            let registry = registry.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let step_start = Instant::now();
                let outcome = if let Some(local_ctx) = local_ctx {
                    // Iterate: ctx snapshot taken before submission; elements run
                    // in order so `as_var` keeps its stateful semantics (P18).
                    match run_iterate(&executor, &guardian, &registry, &step, &local_ctx).await {
                        Ok(out) => StepOutcome::Completed {
                            step_id: step.id.clone(),
                            output: out,
                            duration_ms: step_start.elapsed().as_millis() as u64,
                        },
                        Err(e) => StepOutcome::Failed {
                            step_id: step.id.clone(),
                            reason: e.to_string(),
                            duration_ms: step_start.elapsed().as_millis() as u64,
                        },
                    }
                } else {
                    let resolved = prepared.expect("non-iterate steps carry resolved input");
                    match run_single_step(&executor, &guardian, &registry, &step, &resolved).await {
                        Ok(out) => StepOutcome::Completed {
                            step_id: step.id.clone(),
                            output: out,
                            duration_ms: step_start.elapsed().as_millis() as u64,
                        },
                        Err(e) => StepOutcome::Failed {
                            step_id: step.id.clone(),
                            reason: e.to_string(),
                            duration_ms: step_start.elapsed().as_millis() as u64,
                        },
                    }
                };
                let _ = tx.send(outcome).await;
            });
        }

        if in_flight == 0 {
            break;
        }

        match rx.recv().await {
            Some(o) => {
                in_flight -= 1;
                if process_outcome(
                    o,
                    &shared,
                    workflow,
                    &dependents,
                    &store,
                    &cache,
                    &mut ready,
                )? {
                    stopped = true;
                }
            }
            None => {
                // A spawned task dropped the sender without reporting — i.e. it
                // panicked. Map to InternalError rather than hanging or silently
                // returning a half-finished run.
                return Err(WorkflowError::InternalError {
                    reason:
                        "scheduler channel closed: a step task failed without reporting its outcome"
                            .into(),
                });
            }
        }
    }

    // Finalize the run record.
    let mut g = shared.lock().unwrap();
    if g.failed.is_none() && !g.terminated {
        g.run.finish(RunStatus::Completed);
        store.update_run(&g.run)?;
    }
    if let Some((sid, reason)) = &g.failed {
        return Err(WorkflowError::StepFailed {
            step_id: sid.clone(),
            reason: reason.clone(),
        });
    }
    let result = WorkflowRunResult {
        run_id: g.run.run_id.clone(),
        workflow_name: workflow.name.clone(),
        outputs: g.outputs.clone(),
        steps: {
            // Sort by topological order so callers (and the existing P17/P18
            // tests that index `steps`) see a stable sequence, even though
            // steps actually complete in concurrent (completion) order.
            let mut s = g.step_results.clone();
            s.sort_by_key(|r| *topo.index.get(&r.step_id).unwrap());
            s
        },
        duration_ms: start.elapsed().as_millis() as u64,
        terminated: g.terminated,
        skipped_steps: g.skipped_count,
    };
    Ok(result)
}

/// Apply a single `StepOutcome` to the shared state. Returns `true` if the
/// driver must stop scheduling further steps (a failure or an `end` hit).
fn process_outcome(
    outcome: StepOutcome,
    shared: &Arc<Mutex<Shared>>,
    workflow: &Workflow,
    dependents: &HashMap<String, Vec<String>>,
    store: &Arc<RunStore>,
    cache: &CacheStore,
    ready: &mut VecDeque<String>,
) -> WorkflowResult<bool> {
    let mut g = shared.lock().unwrap();
    match outcome {
        StepOutcome::Failed {
            step_id,
            reason,
            duration_ms,
        } => {
            let skill = workflow.get_step(&step_id).expect("valid id").skill.clone();
            finish_failed(&mut g, store, &step_id, &reason, duration_ms, &skill)?;
            Ok(true)
        }
        StepOutcome::Completed {
            step_id,
            output,
            duration_ms,
        } => {
            let step = workflow.get_step(&step_id).expect("valid id");

            // Defensive: if an upstream was skipped *after* this step was
            // submitted, treat the result as skipped (never as valid). Reuses
            // the same propagation routine as the submit path so the two can
            // never drift apart.
            if step.depends_on.iter().any(|d| g.skipped.contains(d)) {
                if !g.skipped.contains(&step_id) {
                    mark_and_propagate_skip(&mut g, workflow, &step_id, dependents, store)?;
                }
                return Ok(false);
            }

            // Normal completion: write ctx + disk + run record.
            g.ctx.set_step_output(&step_id, output.clone());
            g.outputs.insert(step_id.clone(), output.clone());
            let output_path = store.write_step_output(&g.run.run_id, &step_id, &output)?;
            g.run.steps.push(StepRecord {
                step_id: step_id.clone(),
                skill: step.skill.clone(),
                status: RunStatus::Completed,
                duration_ms,
                error: None,
                output_path,
            });
            g.step_results.push(StepResult {
                step_id: step_id.clone(),
                skill: step.skill.clone(),
                output: output.clone(),
                duration_ms,
            });

            // P20: write a cache entry — but only for a *real* execution. A
            // cache-served outcome (synthesized in the submit path) already has
            // a valid entry and must not overwrite it. The key here is recomputed
            // against the now-final ctx, which equals the one computed at submit
            // time, so the stored key and a future hit agree.
            if !g.cache_served.remove(&step_id) {
                if let Some(key) = g.cache_keys.get(&step_id).cloned() {
                    let upstream_hash = upstream_output_hash(&step.depends_on, &g.outputs);
                    let resolved_hash = if let Some(iter_ref) = &step.iterate {
                        let coll = resolve_value(&Value::String(iter_ref.clone()), &g.ctx)
                            .unwrap_or(Value::Null);
                        resolved_inputs_hash(None, Some(&coll), &step.input.to_string())
                    } else {
                        let resolved = resolve_value(&step.input, &g.ctx).unwrap_or(Value::Null);
                        resolved_inputs_hash(Some(&resolved), None, "")
                    };
                    let entry = make_valid_entry(
                        key,
                        &step_id,
                        CacheKind::StepOutput,
                        &resolved_hash,
                        &upstream_hash,
                        &workflow.schema_version,
                        output.clone(),
                    );
                    // Best-effort: a cache write failure must not fail the run.
                    let _ = cache.put(entry);
                }
            }

            // `vars` assignment (P18 fork ①): evaluated against the ctx and
            // written into `${variables.<name>}`.
            if let Some(vars) = &step.vars {
                for (name, tmpl) in vars {
                    let v = resolve_value(&Value::String(tmpl.clone()), &g.ctx)?;
                    g.ctx.set_variable(name, v);
                }
            }

            // `end` early termination (P18): hit after this step -> stop.
            if let Some(end) = &workflow.end {
                if evaluate_condition(&end.if_expr, &g.ctx)? {
                    g.run.finish(RunStatus::Terminated);
                    g.terminated = true;
                    store.update_run(&g.run)?;
                    return Ok(true);
                }
            }

            // Decrement dependents; enqueue any whose indegree reached 0.
            if let Some(deps) = dependents.get(&step_id) {
                for d in deps {
                    if let Some(r) = g.remaining.get_mut(d) {
                        *r -= 1;
                        if *r == 0 {
                            ready.push_back(d.clone());
                        }
                    }
                }
            }

            store.update_run(&g.run)?;
            Ok(false)
        }
    }
}

/// Record a step failure and mark the whole run `Failed` (Abort strategy).
/// Shared by the driver loop (input-resolution failure) and `process_outcome`
/// (task-reported failure) so both produce identical records.
fn finish_failed(
    g: &mut MutexGuard<'_, Shared>,
    store: &Arc<RunStore>,
    step_id: &str,
    reason: &str,
    duration_ms: u64,
    skill: &str,
) -> WorkflowResult<()> {
    g.run.steps.push(StepRecord {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: RunStatus::Failed,
        duration_ms,
        error: Some(reason.to_string()),
        output_path: PathBuf::new(),
    });
    g.run.finish(RunStatus::Failed);
    g.failed = Some((step_id.to_string(), reason.to_string()));
    store.update_run(&g.run)?;
    Ok(())
}

/// Mark `sid` skipped and transitively propagate the skip to its dependents.
/// A skip is not a failure (P18 fork ③).
fn mark_and_propagate_skip(
    g: &mut MutexGuard<'_, Shared>,
    workflow: &Workflow,
    sid: &str,
    dependents: &HashMap<String, Vec<String>>,
    store: &Arc<RunStore>,
) -> WorkflowResult<()> {
    // Destructure once: `record_skipped` needs two disjoint `&mut` fields, which
    // the borrow checker only accepts through a split borrow (not two `&mut g.*`).
    let state = &mut **g;
    let skill = workflow.get_step(sid).expect("valid id").skill.clone();
    record_skipped(&mut state.run, &mut state.step_results, sid, &skill);
    if state.skipped.insert(sid.to_string()) {
        state.skipped_count += 1;
    }
    let mut stack = vec![sid.to_string()];
    while let Some(x) = stack.pop() {
        if let Some(deps) = dependents.get(&x) {
            for d in deps {
                if state.skipped.insert(d.clone()) {
                    let sk = workflow.get_step(d).expect("valid id").skill.clone();
                    record_skipped(&mut state.run, &mut state.step_results, d, &sk);
                    state.skipped_count += 1;
                    stack.push(d.clone());
                }
            }
        }
    }
    store.update_run(&g.run)?;
    Ok(())
}

/// Execute a single non-iterate step: lookup skill -> Executor -> Guardian
/// validation -> parse stdout as JSON. Equivalent to `engine::execute_step`
/// (which is removed to avoid duplication) but takes collaborators by ref so it
/// can run inside a spawned task.
async fn run_single_step(
    executor: &Executor,
    guardian: &Guardian,
    registry: &SkillRegistry,
    step: &WorkflowStep,
    input: &Value,
) -> WorkflowResult<Value> {
    let skill = registry
        .get(&step.skill)
        .ok_or_else(|| WorkflowError::SkillNotFound {
            skill_name: step.skill.clone(),
            step_id: step.id.clone(),
        })?;
    let exec_result = executor.execute(&skill, input).await?;
    guardian.validate_once(&skill, &exec_result.stdout).await?;
    let output: Value =
        serde_json::from_str(&exec_result.stdout).map_err(|e| WorkflowError::ParseError {
            path: format!("step:{}:stdout", step.id),
            reason: e.to_string(),
        })?;
    Ok(output)
}

/// Execute an `iterate` step: loop over the collection, binding each element
/// to `as_var` (default `item`), resolve the input per element, and collect
/// outputs into an array (P18 fork ④). Elements run in order — `as_var` carries
/// per-element state, so this is intentionally sequential within the step.
async fn run_iterate(
    executor: &Executor,
    guardian: &Guardian,
    registry: &SkillRegistry,
    step: &WorkflowStep,
    local_ctx: &ExpressionContext,
) -> WorkflowResult<Value> {
    let iter_ref = step
        .iterate
        .as_ref()
        .expect("run_iterate called only when iterate is Some");
    let collection = resolve_value(&Value::String(iter_ref.clone()), local_ctx)?;
    let items = collection
        .as_array()
        .ok_or_else(|| WorkflowError::ExpressionResolution {
            reason: format!("iterate target `{iter_ref}` is not an array"),
        })?;
    let as_var = step.as_var.clone().unwrap_or_else(|| "item".to_string());
    let mut collected: Vec<Value> = Vec::with_capacity(items.len());
    for item in items {
        let mut c = local_ctx.clone();
        c.set_variable(&as_var, item.clone());
        let per_input = resolve_value(&step.input, &c)?;
        let out = run_single_step(executor, guardian, registry, step, &per_input).await?;
        collected.push(out);
    }
    Ok(Value::Array(collected))
}
