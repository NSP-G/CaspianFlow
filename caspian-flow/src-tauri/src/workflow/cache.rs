//! Intermediate-result cache (P20).
//!
//! Solves one problem: given identical inputs, a downstream step may skip
//! re-execution and reuse an upstream output that was already computed.
//!
//! ## Hit semantics — deterministic, never probabilistic
//!
//! A cache entry is addressed by a key that is the SHA-256 of five
//! runtime-derived fields:
//!
//! ```text
//! cache_key = sha256(step_id ‖ kind ‖ resolved_inputs_hash ‖ upstream_output_hash ‖ impl_version)
//! ```
//!
//! A hit requires the key to match **exactly**. There is no similarity,
//! fuzziness, or ordering — that is deliberate. (An earlier measurement of
//! cosine-similarity thresholds showed they cannot separate "equivalent
//! rewrites" from "directionally-opposite operations"; see
//! `P20_SIMILARITY_THRESHOLD_MEASUREMENT.md`.) Execution caching must be
//! correct by construction, so the key is a pure function of the step's
//! deterministic inputs.
//!
//! ## Storage location — a dedicated domain, never `temp`
//!
//! The doc draft placed the index at `temp/workflows/<run_id>/.cache_index.json`.
//! That is wrong for two reasons: (1) `run_id` is unique per execution
//! (`run_{nanos}`), so a per-run index can never be reused across runs — which
//! is the *only* value an intermediate-result cache adds (within a run,
//! downstream steps already receive upstream outputs via the in-memory
//! context); and (2) `temp` is ephemeral and may be cleaned between runs, while
//! a cache must outlive a single run (TTL / version based). The index therefore
//! lives in a **separate, persistent cache domain**, rooted per workflow:
//!
//! ```text
//! ~/.caspian/cache/workflows/<workflow_name>/index.json
//! ```
//!
//! `<workflow_name>` is the workflow's `name`; the cache namespace is per
//! workflow (cross-workflow sharing is forbidden by design).
//!
//! ## Invalidation — three triggers, all explicit
//!
//! 1. **By version**: on run start, if the index header `schema_version`
//!    differs from the current `workflow.schema_version`, the whole index is
//!    marked `stale`.
//! 2. **By dependency**: *structural and automatic*. `upstream_output_hash`
//!    is part of the key, so any change in an upstream's output changes the
//!    downstream key → automatic miss. No reverse-dependency table needed.
//! 3. **By time (TTL)**: `get()` treats `now > ttl` as `stale` and does not
//!    serve it. Expired entries are pruned on load (best-effort archival).
//!
//! The `pending` lifecycle state from the doc is intentionally omitted: the key
//! is computed on demand at submit time, so pre-creating `pending` entries
//! adds bookkeeping with no functional benefit. Only `valid` / `stale` /
//! `archived` are material.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::types::{WorkflowError, WorkflowResult};

/// Discriminator for what kind of value an entry caches. `kind` is `StepOutput`
/// for every entry today (there is no `kind` field on `WorkflowStep`); the enum
/// exists for forward-compatibility with other cacheable entities — variable
/// snapshots, workflow results, computed values — without widening the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKind {
    /// A normal step's execution output.
    StepOutput,
    // Reserved for future use: WorkflowResult, ComputedValue, ...
}

impl CacheKind {
    /// Stable string used inside the cache key and on disk.
    pub fn as_str(&self) -> &'static str {
        match self {
            CacheKind::StepOutput => "step_output",
        }
    }
}

/// Default TTL for a cache entry, in seconds (7 days).
pub const DEFAULT_TTL_SECS: u64 = 7 * 24 * 3600;

/// Separator used inside the key string. NUL cannot appear in any field.
const SEP: char = '\u{0}';

/// Lifecycle status of a cache entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    /// Produced by a real execution; can be served to downstream steps.
    Valid,
    /// No longer trustworthy (version change / TTL expiry). Must not be served.
    Stale,
    /// Pruned from disk; recorded only for completeness of the model.
    Archived,
}

/// A single cached step output, persisted as one entry in the workflow index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub cache_key: String,
    pub step_id: String,
    pub kind: CacheKind,
    pub resolved_inputs_hash: String,
    pub upstream_output_hash: String,
    pub impl_version: String,
    /// Unix seconds when the entry was produced.
    pub executed_at: u64,
    /// Unix seconds when the entry expires (TTL).
    pub ttl: u64,
    pub status: CacheStatus,
    /// The cached output value. Inlined so a hit is one file read.
    pub output: Value,
}

/// On-disk index: a map from `cache_key` to its entry, plus a header recording
/// the `schema_version` the entries were produced under (for version invalidation).
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheIndex {
    schema_version: String,
    entries: BTreeMap<String, CacheEntry>,
}

/// Workflow-level cache store. All mutation is file-backed and single-threaded
/// per store instance; the scheduler owns one for the duration of a run.
pub struct CacheStore {
    dir: PathBuf,
}

impl CacheStore {
    /// Root the store at `<base_dir>/<workflow_name>` — one `index.json` per
    /// workflow, inside the dedicated cache domain (never under `temp`). The
    /// scheduler passes `RunStore::cache_root()` (i.e. `<cache>/workflows`).
    pub fn new(base_dir: &Path, workflow_name: &str) -> Self {
        Self {
            dir: base_dir.join(workflow_name),
        }
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    fn load(&self) -> CacheIndex {
        let path = self.index_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<CacheIndex>(&contents) {
                Ok(mut idx) => {
                    // Best-effort archival: drop entries whose TTL already expired.
                    // They would miss anyway; pruning keeps the index small.
                    idx.entries
                        .retain(|_, e| e.status != CacheStatus::Stale && now_secs() < e.ttl);
                    idx
                }
                Err(_) => CacheIndex::default(),
            },
            Err(_) => CacheIndex::default(),
        }
    }

    fn save(&self, idx: &CacheIndex) -> WorkflowResult<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| WorkflowError::ParseError {
            path: self.dir.display().to_string(),
            reason: e.to_string(),
        })?;
        let json = serde_json::to_string_pretty(idx).map_err(|e| WorkflowError::ParseError {
            path: self.index_path().display().to_string(),
            reason: e.to_string(),
        })?;
        std::fs::write(self.index_path(), json).map_err(|e| WorkflowError::ParseError {
            path: self.index_path().display().to_string(),
            reason: e.to_string(),
        })?;
        Ok(())
    }

    /// Version invalidation (doc trigger #1). If the index was produced under a
    /// different `schema_version`, mark every surviving entry `stale`. Returns
    /// `true` if a version mismatch was detected (i.e. the cache was invalidated).
    pub fn invalidate_if_version_changed(&self, current_version: &str) -> WorkflowResult<bool> {
        let mut idx = self.load();
        if idx.schema_version == current_version {
            return Ok(false);
        }
        for e in idx.entries.values_mut() {
            e.status = CacheStatus::Stale;
        }
        idx.schema_version = current_version.to_string();
        self.save(&idx)?;
        Ok(true)
    }

    /// Look up an entry by key. Returns `None` if absent, `stale`, or expired
    /// (TTL). Only `valid` and unexpired entries are served — never silently.
    pub fn get(&self, key: &str) -> Option<CacheEntry> {
        let idx = self.load();
        match idx.entries.get(key) {
            Some(e) if e.status == CacheStatus::Valid && now_secs() < e.ttl => Some(e.clone()),
            _ => None,
        }
    }

    /// Insert or overwrite an entry as `valid`.
    pub fn put(&self, entry: CacheEntry) -> WorkflowResult<()> {
        let mut idx = self.load();
        idx.schema_version = entry.impl_version.clone();
        idx.entries.insert(entry.cache_key.clone(), entry);
        self.save(&idx)
    }

    /// Number of entries currently tracked (for tests / diagnostics).
    pub fn entry_count(&self) -> usize {
        self.load().entries.len()
    }
}

// ---------------------------------------------------------------------------
// Key construction
// ---------------------------------------------------------------------------

/// Deterministically hash a `serde_json::Value` by first canonicalizing it
/// (recursively sorting object keys) so equal logical values hash equally
/// across runs — `serde_json::Value::to_string` preserves insertion order and
/// would otherwise produce different digests for the same data.
pub fn hash_value(value: &Value) -> String {
    sha256_hex(&canonical_json(value))
}

/// Hash an arbitrary string (templates, ids).
pub fn hash_str(s: &str) -> String {
    sha256_hex(s)
}

/// Build the cache key from its five components.
pub fn cache_key_of(
    step_id: &str,
    kind: &str,
    resolved_inputs_hash: &str,
    upstream_output_hash: &str,
    impl_version: &str,
) -> String {
    sha256_hex(&format!(
        "{sep}{step_id}{sep}{kind}{sep}{resolved_inputs_hash}{sep}{upstream_output_hash}{sep}{impl_version}{sep}",
        sep = SEP
    ))
}

/// Hash the combined outputs of a step's upstreams. Deterministic: upstream ids
/// are sorted before hashing. Safe to call only after every upstream has
/// completed (callers guarantee this — a step enters the ready queue only once
/// all upstreams wrote their output).
pub fn upstream_output_hash(upstream_ids: &[String], outputs: &HashMap<String, Value>) -> String {
    let mut ids = upstream_ids.to_vec();
    ids.sort();
    let mut s = String::new();
    for id in &ids {
        s.push_str(id);
        s.push(SEP);
        // Upstream guaranteed present; fall back to Null only to stay total.
        let v = outputs.get(id).unwrap_or(&Value::Null);
        s.push_str(&hash_value(v));
        s.push(SEP);
    }
    sha256_hex(&s)
}

/// Derive the `resolved_inputs_hash` for a step.
///
/// - Non-iterate: `resolved` is the already-resolved input `Value`.
/// - Iterate: `resolved` is `None`; pass the resolved *collection* and the raw
///   input template string instead (the per-element expansion happens inside
///   the spawned task and is not part of the key — see pre-check F2).
pub fn resolved_inputs_hash(
    resolved: Option<&Value>,
    collection: Option<&Value>,
    input_template: &str,
) -> String {
    match resolved {
        Some(v) => hash_value(v),
        None => {
            let coll = collection.map(hash_value).unwrap_or_default();
            sha256_hex(&format!(
                "{sep}{coll}{sep}{tpl}{sep}",
                sep = SEP,
                tpl = input_template
            ))
        }
    }
}

/// Convenience: build a `valid` entry for a just-executed step.
pub fn make_valid_entry(
    key: String,
    step_id: &str,
    kind: CacheKind,
    resolved_inputs_hash: &str,
    upstream_output_hash: &str,
    impl_version: &str,
    output: Value,
) -> CacheEntry {
    let now = now_secs();
    CacheEntry {
        cache_key: key,
        step_id: step_id.to_string(),
        kind,
        resolved_inputs_hash: resolved_inputs_hash.to_string(),
        upstream_output_hash: upstream_output_hash.to_string(),
        impl_version: impl_version.to_string(),
        executed_at: now,
        ttl: now + DEFAULT_TTL_SECS,
        status: CacheStatus::Valid,
        output,
    }
}

// ---------------------------------------------------------------------------
// Canonicalization + hashing helpers
// ---------------------------------------------------------------------------

/// Recursively sort object keys so the serialized form is independent of
/// insertion order — the foundation of reproducible cross-run hashing.
fn canonical_json(value: &Value) -> String {
    fn norm(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let sorted: BTreeMap<String, Value> =
                    map.iter().map(|(k, val)| (k.clone(), norm(val))).collect();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(arr) => Value::Array(arr.iter().map(norm).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&norm(value)).unwrap_or_else(|_| "null".to_string())
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Current unix timestamp in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;

    fn temp_cache(name: &str) -> (tempfile::TempDir, CacheStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::new(dir.path(), name);
        (dir, store)
    }

    #[test]
    fn test_canonical_hash_ignores_key_order() {
        // F3: same logical value, different insertion order -> same hash.
        let a = serde_json::json!({"b": 1, "a": 2});
        let b = serde_json::json!({"a": 2, "b": 1});
        assert_eq!(hash_value(&a), hash_value(&b));
        // And a genuinely different value differs.
        let c = serde_json::json!({"a": 2, "b": 3});
        assert_ne!(hash_value(&a), hash_value(&c));
    }

    #[test]
    fn test_cache_key_is_deterministic_and_order_independent() {
        let k1 = cache_key_of("s1", "step_output", "h_in", "h_up", "1.0");
        let k2 = cache_key_of("s1", "step_output", "h_in", "h_up", "1.0");
        assert_eq!(k1, k2);
        // Any field change breaks the key (structural, deterministic invalidation).
        assert_ne!(k1, cache_key_of("s1", "step_output", "h_in", "h_up", "1.1"));
        assert_ne!(k1, cache_key_of("s2", "step_output", "h_in", "h_up", "1.0"));

        // upstream_output_hash must not depend on the order upstream ids are listed.
        let mut outs = StdHashMap::new();
        outs.insert("a".to_string(), serde_json::json!(1));
        outs.insert("b".to_string(), serde_json::json!(2));
        let u1 = upstream_output_hash(&["a".to_string(), "b".to_string()], &outs);
        let u2 = upstream_output_hash(&["b".to_string(), "a".to_string()], &outs);
        assert_eq!(u1, u2);
        // Different upstream output -> different hash.
        let mut outs2 = StdHashMap::new();
        outs2.insert("a".to_string(), serde_json::json!(1));
        outs2.insert("b".to_string(), serde_json::json!(99));
        assert_ne!(
            u1,
            upstream_output_hash(&["a".to_string(), "b".to_string()], &outs2)
        );
    }

    #[test]
    fn test_resolved_inputs_hash_iterate_vs_scalar() {
        let scalar = resolved_inputs_hash(Some(&serde_json::json!({"x": 1})), None, "");
        let iterate = resolved_inputs_hash(None, Some(&serde_json::json!([1, 2, 3])), "template");
        assert_ne!(scalar, iterate);
        // Iterating twice over the same collection+template is stable.
        let iterate2 = resolved_inputs_hash(None, Some(&serde_json::json!([1, 2, 3])), "template");
        assert_eq!(iterate, iterate2);
    }

    #[test]
    fn test_put_get_roundtrip_and_expiry() {
        let (_dir, store) = temp_cache("wf");
        let entry = make_valid_entry(
            "key1".into(),
            "s1",
            CacheKind::StepOutput,
            "in",
            "up",
            "1.0",
            serde_json::json!({"v": 7}),
        );
        store.put(entry).unwrap();
        let got = store.get("key1").expect("valid entry should be served");
        assert_eq!(got.output, serde_json::json!({"v": 7}));
        assert_eq!(got.status, CacheStatus::Valid);

        // TTL expiry: rewrite the stored index with a past ttl, then get() must
        // refuse to serve it (best-effort archival prunes it on load).
        let path = store.dir.join("index.json");
        let mut idx: CacheIndex =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for e in idx.entries.values_mut() {
            e.ttl = 1; // far in the past
        }
        std::fs::write(&path, serde_json::to_string_pretty(&idx).unwrap()).unwrap();
        assert!(
            store.get("key1").is_none(),
            "expired entry must not be served"
        );
    }

    #[test]
    fn test_version_invalidation_marks_stale() {
        let (_dir, store) = temp_cache("wf");
        let entry = make_valid_entry(
            "key1".into(),
            "s1",
            CacheKind::StepOutput,
            "in",
            "up",
            "1.0",
            serde_json::json!(1),
        );
        store.put(entry).unwrap();
        assert!(store.get("key1").is_some());

        // Bumping the schema version invalidates the whole index.
        let changed = store.invalidate_if_version_changed("2.0").unwrap();
        assert!(changed);
        assert!(
            store.get("key1").is_none(),
            "stale entry must not be served"
        );

        // A second call with the same version is a no-op (already invalidated).
        let changed_again = store.invalidate_if_version_changed("2.0").unwrap();
        assert!(!changed_again);

        // An entry produced under the new version is served normally.
        let entry2 = make_valid_entry(
            "key2".into(),
            "s1",
            CacheKind::StepOutput,
            "in",
            "up",
            "2.0",
            serde_json::json!(2),
        );
        store.put(entry2).unwrap();
        assert!(store.get("key2").is_some());
    }
}
