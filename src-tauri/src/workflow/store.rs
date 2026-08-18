//! Run persistence for workflow execution.
//!
//! Stores run state under `<temp>/workflows/<run_id>/`:
//!
//! ```text
//! <temp>/workflows/<run_id>/
//!   run.json              # RunRecord (metadata + per-step records)
//!   steps/<step_id>.json  # each step's output (serde_json::Value)
//! ```
//!
//! P17 implements **basic per-run storage** only — no reference counting or
//! LRU eviction (that is P20). Downstream steps read upstream outputs from
//! the in-memory execution context; the on-disk files are the durable record
//! and the basis for [`RunStore::list_runs`] / [`RunStore::get_run`] history
//! queries.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::CaspianPaths;
use crate::types::{WorkflowError, WorkflowResult};

/// Lifecycle status of a workflow run or a single step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Execution started, not yet finished.
    Running,
    /// All steps completed successfully.
    Completed,
    /// A step failed and the run aborted.
    Failed,
    /// Step skipped because its `if` condition was false (P18, not a failure).
    Skipped,
    /// Run stopped early by an `end` condition (P18, intentional, not a failure).
    Terminated,
}

/// A single step's persisted record within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub step_id: String,
    pub skill: String,
    pub status: RunStatus,
    pub duration_ms: u64,
    pub error: Option<String>,
    /// Path to the step's output JSON file (empty if the step failed before producing output).
    pub output_path: PathBuf,
}

/// Metadata record for one workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub workflow_name: String,
    pub status: RunStatus,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub steps: Vec<StepRecord>,
}

impl RunRecord {
    /// Mark the run finished with the given status and record the finish time.
    pub fn finish(&mut self, status: RunStatus) {
        self.status = status;
        self.finished_at = Some(now_secs());
    }
}

/// Capacity of the bounded recency cache used by [`RunStore::get_run`] (P35).
///
/// Bounds the in-memory footprint of run records: at most this many full
/// `RunRecord`s (each carrying a `Vec<StepRecord>`) are retained at once,
/// regardless of how many thousands of runs exist on disk. Repeated lookups of
/// a recently-accessed run — e.g. the UI polling a completed run — are served
/// from here instead of re-parsing `run.json` off disk every time.
pub const RUN_CACHE_CAPACITY: usize = 256;

/// Persistent store for workflow run state.
///
/// Constructed from the CaspianFlow `temp` directory; all runs live under
/// `<temp>/workflows/`.
pub struct RunStore {
    workflows_dir: PathBuf,
    /// Root for the P20 intermediate-result cache (`<cache>/workflows`).
    /// A *separate* domain from `workflows_dir` (which lives under `temp` and
    /// may be cleaned between runs); the cache must outlive a single run.
    cache_dir: PathBuf,
    /// Bounded recency cache of full `RunRecord`s, keyed by `run_id` (P35).
    /// `(recency_queue, id -> record)`; the queue front is most-recently-used.
    recent: Mutex<(VecDeque<String>, HashMap<String, RunRecord>)>,
}

impl RunStore {
    /// Create a store rooted at `<temp_dir>/workflows`.
    pub fn new(temp_dir: &Path) -> Self {
        Self {
            workflows_dir: temp_dir.join("workflows"),
            cache_dir: temp_dir.join("cache").join("workflows"),
            recent: Mutex::new((VecDeque::new(), HashMap::new())),
        }
    }

    /// Create a store from the resolved CaspianFlow paths. The run store lives
    /// under `temp`, but the P20 cache lives under `cache` — distinct domains.
    pub fn from_paths(paths: &CaspianPaths) -> Self {
        Self {
            workflows_dir: paths.temp.join("workflows"),
            cache_dir: paths.cache.join("workflows"),
            recent: Mutex::new((VecDeque::new(), HashMap::new())),
        }
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.workflows_dir.join(run_id)
    }

    /// Root directory for all runs (`<temp>/workflows`). Run state is ephemeral
    /// and may be cleaned between runs.
    pub fn workflows_root(&self) -> &Path {
        &self.workflows_dir
    }

    /// Root directory for the P20 intermediate-result cache (`<cache>/workflows`).
    /// Lives in a *separate* domain from [`RunStore::workflows_root`] so cached
    /// entries survive run cleanup. The scheduler roots a
    /// [`crate::workflow::cache::CacheStore`] here as
    /// `<cache_root>/<workflow_name>/index.json`.
    pub fn cache_root(&self) -> &Path {
        &self.cache_dir
    }

    fn steps_dir(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("steps")
    }

    fn run_json_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("run.json")
    }

    /// Create a new run and return its record (persists `run.json`).
    pub fn create_run(&self, workflow_name: &str) -> WorkflowResult<RunRecord> {
        let run_id = new_run_id();
        let record = RunRecord {
            run_id: run_id.clone(),
            workflow_name: workflow_name.to_string(),
            status: RunStatus::Running,
            started_at: now_secs(),
            finished_at: None,
            steps: Vec::new(),
        };
        std::fs::create_dir_all(self.steps_dir(&run_id))?;
        self.write_run_record(&record)?;
        self.invalidate_cache(&run_id);
        Ok(record)
    }

    /// Persist a single step's output to `<run_id>/steps/<step_id>.json`.
    pub fn write_step_output(
        &self,
        run_id: &str,
        step_id: &str,
        output: &Value,
    ) -> WorkflowResult<PathBuf> {
        let path = self.steps_dir(run_id).join(format!("{step_id}.json"));
        let json = serde_json::to_string_pretty(output).map_err(|e| WorkflowError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a step's output from disk.
    pub fn read_step_output(&self, run_id: &str, step_id: &str) -> WorkflowResult<Value> {
        let path = self.steps_dir(run_id).join(format!("{step_id}.json"));
        let contents = std::fs::read_to_string(&path).map_err(|e| WorkflowError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        serde_json::from_str(&contents).map_err(|e| WorkflowError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Persist the run metadata record (status, finished_at, steps).
    pub fn update_run(&self, record: &RunRecord) -> WorkflowResult<()> {
        // Drop any cached copy so a subsequent get_run re-reads the fresh record.
        self.invalidate_cache(&record.run_id);
        self.write_run_record(record)
    }

    fn write_run_record(&self, record: &RunRecord) -> WorkflowResult<()> {
        let path = self.run_json_path(&record.run_id);
        let json = serde_json::to_string_pretty(record).map_err(|e| WorkflowError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        std::fs::write(&path, json).map_err(|e| WorkflowError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        Ok(())
    }

    /// Get a single run record by id (`None` if it does not exist).
    ///
    /// Serves from the bounded recency cache on a hit; otherwise reads
    /// `run.json` off disk and caches the result (P35).
    pub fn get_run(&self, run_id: &str) -> WorkflowResult<Option<RunRecord>> {
        if let Some(cached) = self.lookup_cache(run_id) {
            return Ok(Some(cached));
        }
        let path = self.run_json_path(run_id);
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path).map_err(|e| WorkflowError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        let record: RunRecord = serde_json::from_str(&contents).map_err(|e| WorkflowError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        self.insert_cache(run_id.to_string(), record.clone());
        Ok(Some(record))
    }

    /// List up to `limit` historical run records, newest first.
    ///
    /// Bounds the in-memory result so callers (the UI history view) never load
    /// thousands of `RunRecord`s at once. For the full, unbounded list use
    /// [`RunStore::list_runs`].
    pub fn list_runs_limited(&self, limit: usize) -> WorkflowResult<Vec<RunRecord>> {
        let mut runs = self.list_runs()?;
        runs.truncate(limit);
        Ok(runs)
    }

    // --- Bounded recency cache (P35) -------------------------------------

    /// Return a clone of the cached record if present, refreshing its recency.
    fn lookup_cache(&self, run_id: &str) -> Option<RunRecord> {
        let mut cache = self.recent.lock().unwrap();
        let rec = cache.1.get(run_id).cloned()?;
        // Refresh recency now that the immutable borrow of `cache.1` is dropped.
        if let Some(pos) = cache.0.iter().position(|k| k == run_id) {
            cache.0.remove(pos);
        }
        cache.0.push_front(run_id.to_string());
        Some(rec)
    }

    /// Insert (or replace) a record and evict the least-recently-used entry
    /// when over [`RUN_CACHE_CAPACITY`].
    fn insert_cache(&self, run_id: String, record: RunRecord) {
        let mut cache = self.recent.lock().unwrap();
        cache.1.insert(run_id.clone(), record);
        cache.0.push_front(run_id);
        if cache.0.len() > RUN_CACHE_CAPACITY {
            if let Some(old) = cache.0.pop_back() {
                cache.1.remove(&old);
            }
        }
    }

    /// Remove any cached copy of `run_id` (called on create/update so the cache
    /// never serves a stale record).
    fn invalidate_cache(&self, run_id: &str) {
        let mut cache = self.recent.lock().unwrap();
        if let Some(pos) = cache.0.iter().position(|k| k == run_id) {
            cache.0.remove(pos);
        }
        cache.1.remove(run_id);
    }

    /// List all historical run records, newest first.
    pub fn list_runs(&self) -> WorkflowResult<Vec<RunRecord>> {
        if !self.workflows_dir.exists() {
            return Ok(Vec::new());
        }
        let mut runs: Vec<RunRecord> = Vec::new();
        for entry in std::fs::read_dir(&self.workflows_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let run_json = path.join("run.json");
            if !run_json.exists() {
                continue;
            }
            if let Ok(contents) = std::fs::read_to_string(&run_json) {
                if let Ok(record) = serde_json::from_str::<RunRecord>(&contents) {
                    runs.push(record);
                }
            }
        }
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(runs)
    }
}

/// Current unix timestamp in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Generate a unique run id from the high-resolution clock.
fn new_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("run_{nanos:x}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, RunStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn test_create_run() {
        let (_dir, store) = temp_store();
        let run = store.create_run("my_wf").unwrap();
        assert_eq!(run.workflow_name, "my_wf");
        assert_eq!(run.status, RunStatus::Running);
        assert!(run.finished_at.is_none());
        assert!(store.run_json_path(&run.run_id).exists());
    }

    #[test]
    fn test_write_read_step_output() {
        let (_dir, store) = temp_store();
        let run = store.create_run("wf").unwrap();
        let output = serde_json::json!({"content": "hello", "n": 42});
        let path = store
            .write_step_output(&run.run_id, "step_a", &output)
            .unwrap();
        assert!(path.exists());
        let read = store.read_step_output(&run.run_id, "step_a").unwrap();
        assert_eq!(read, output);
    }

    #[test]
    fn test_read_missing_step_output() {
        let (_dir, store) = temp_store();
        let run = store.create_run("wf").unwrap();
        let err = store.read_step_output(&run.run_id, "nope");
        assert!(err.is_err());
    }

    #[test]
    fn test_update_run_status() {
        let (_dir, store) = temp_store();
        let mut run = store.create_run("wf").unwrap();
        run.finish(RunStatus::Completed);
        store.update_run(&run).unwrap();
        let reloaded = store.get_run(&run.run_id).unwrap().unwrap();
        assert_eq!(reloaded.status, RunStatus::Completed);
        // `finish()` stamps the completion time — it must survive the round-trip.
        assert!(reloaded.finished_at.is_some());
    }

    #[test]
    fn test_get_run_missing() {
        let (_dir, store) = temp_store();
        assert!(store.get_run("does_not_exist").unwrap().is_none());
    }

    #[test]
    fn test_list_runs_newest_first() {
        let (_dir, store) = temp_store();
        // Write two runs directly with distinct started_at (private helper visible here).
        let older = RunRecord {
            run_id: "run_old".into(),
            workflow_name: "wf".into(),
            status: RunStatus::Completed,
            started_at: 1000,
            finished_at: Some(1001),
            steps: vec![],
        };
        let newer = RunRecord {
            run_id: "run_new".into(),
            workflow_name: "wf".into(),
            status: RunStatus::Completed,
            started_at: 2000,
            finished_at: Some(2001),
            steps: vec![],
        };
        // `write_run_record` writes into `<root>/<run_id>/run.json`; the run
        // directory is normally created by `create_run`, so make it here.
        for r in [&older, &newer] {
            std::fs::create_dir_all(store.steps_dir(&r.run_id)).unwrap();
            store.write_run_record(r).unwrap();
        }
        let runs = store.list_runs().unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "run_new");
        assert_eq!(runs[1].run_id, "run_old");
    }

    #[test]
    fn test_list_runs_empty() {
        let (_dir, store) = temp_store();
        assert!(store.list_runs().unwrap().is_empty());
    }

    #[test]
    fn test_persist_path_is_steps() {
        let (_dir, store) = temp_store();
        let run = store.create_run("wf").unwrap();
        let output = serde_json::json!({"ok": true});
        let path = store.write_step_output(&run.run_id, "s1", &output).unwrap();
        let p = path.to_string_lossy();
        assert!(p.contains("/steps/"), "expected /steps/ in {p}");
        assert!(p.ends_with("s1.json"));
        assert!(path.exists());
    }

    #[test]
    fn test_get_run_reflects_update_not_stale_cache() {
        // P35: the bounded recency cache must never serve a record that was
        // overwritten by `update_run`.
        let (_dir, store) = temp_store();
        let mut run = store.create_run("wf").unwrap();
        let first = store.get_run(&run.run_id).unwrap().unwrap();
        assert_eq!(first.status, RunStatus::Running);

        run.finish(RunStatus::Completed);
        store.update_run(&run).unwrap();

        let after = store.get_run(&run.run_id).unwrap().unwrap();
        assert_eq!(after.status, RunStatus::Completed);
    }

    #[test]
    fn test_list_runs_limited_truncates_newest_first() {
        // P35: bounded history reads must respect the limit and stay sorted.
        let (_dir, store) = temp_store();
        for i in 0..5u64 {
            let mut r = store.create_run("wf").unwrap();
            r.started_at = i * 100;
            store.update_run(&r).unwrap();
        }
        let limited = store.list_runs_limited(3).unwrap();
        assert_eq!(limited.len(), 3);
        // Newest (started_at = 400) must be first.
        assert_eq!(limited[0].started_at, 400);
        assert_eq!(limited[2].started_at, 200);
        // Unbounded list still returns everything.
        assert_eq!(store.list_runs().unwrap().len(), 5);
    }
}
