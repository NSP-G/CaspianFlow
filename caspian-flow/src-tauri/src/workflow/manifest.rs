//! Workflow *definition* persistence (P27, 模式 C).
//!
//! This is a distinct domain from the P17 run-state store (`store.rs`, which
//! lives under `temp/workflows/<run_id>/`). Here we own the **source of
//! truth** the user edits in the canvas:
//!
//! - Formal file: `<workflows>/<name>/workflow.yaml` — written **only** on an
//!   explicit save (`Cmd+S` / button), atomically (temp file + rename).
//! - Draft file:  `<workflows>/.drafts/<name>.yaml` — written automatically on
//!   every edit (debounced 500ms by the frontend). The scanner never reads
//!   `.drafts/`, so a crash mid-edit can never corrupt the executable set.
//!
//! Conflict detection: an explicit save may carry the mtime recorded when the
//! workflow was loaded. If the on-disk formal file's mtime has since changed
//! (e.g. an external editor touched it), the save is rejected with
//! [`WorkflowError::Conflict`] instead of silently overwriting.

use std::path::{Path, PathBuf};

use crate::config::CaspianPaths;
use crate::types::{WorkflowError, WorkflowResult};

use super::schema::Workflow;
use super::scanner::{modified_at, WorkflowScanner, WorkflowSummary};

/// A lightweight entry for the workflow list UI.
#[derive(Debug, Clone)]
pub struct WorkflowListEntry {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub modified: u64,
    pub step_count: usize,
}

/// Manifest path for a named workflow: `<root>/<name>/workflow.yaml`.
pub fn manifest_path(root: &Path, name: &str) -> PathBuf {
    root.join(name).join("workflow.yaml")
}

/// Draft path for a named workflow: `<root>/.drafts/<name>.yaml`.
pub fn draft_path(root: &Path, name: &str) -> PathBuf {
    root.join(".drafts").join(format!("{name}.yaml"))
}

/// Ensure the workflow root and the per-workflow directory exist.
fn ensure_dirs(paths: &CaspianPaths, name: &str) -> WorkflowResult<PathBuf> {
    std::fs::create_dir_all(&paths.workflows).map_err(WorkflowError::Io)?;
    let wf_dir = paths.workflows.join(name);
    std::fs::create_dir_all(&wf_dir).map_err(WorkflowError::Io)?;
    Ok(wf_dir)
}

/// Atomically write `yaml` to `target` via a temp file + `rename`.
///
/// `rename` within the same filesystem is atomic, so a crash never leaves a
/// half-written manifest (P27 原子写入 requirement).
fn atomic_write(target: &Path, yaml: &str) -> WorkflowResult<()> {
    let tmp = target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{}.tmp",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workflow")
        ));
    std::fs::write(&tmp, yaml).map_err(WorkflowError::Io)?;
    std::fs::rename(&tmp, target).map_err(WorkflowError::Io)?;
    Ok(())
}

/// List all workflow definitions (skips `.drafts/` and hidden dirs).
pub fn list(paths: &CaspianPaths) -> WorkflowResult<Vec<WorkflowSummary>> {
    WorkflowScanner::from_paths(paths).list()
}

/// Lightweight list entries for the UI (derived from [`list`]).
pub fn list_entries(paths: &CaspianPaths) -> WorkflowResult<Vec<WorkflowListEntry>> {
    Ok(list(paths)?
        .into_iter()
        .map(|s| WorkflowListEntry {
            name: s.dir_name,
            display_name: s.workflow.display_name.clone(),
            description: s.workflow.description.clone(),
            modified: s.modified,
            step_count: s.workflow.step_count(),
        })
        .collect())
}

/// Read a workflow definition and its mtime. [`WorkflowError::NotFound`] if the
/// manifest is absent.
pub fn read_workflow(paths: &CaspianPaths, name: &str) -> WorkflowResult<(Workflow, u64)> {
    let manifest = manifest_path(&paths.workflows, name);
    if !manifest.exists() {
        return Err(WorkflowError::NotFound { name: name.to_string() });
    }
    let workflow = Workflow::load(&manifest)?;
    let modified = modified_at(&manifest)?;
    Ok((workflow, modified))
}

/// Read the raw manifest text and its mtime, without parsing into [`Workflow`].
///
/// Used by the GUI so the canvas's `ui` layout section (ignored by P17) round-
/// trips intact. [`WorkflowError::NotFound`] if the manifest is absent.
pub fn read_raw(paths: &CaspianPaths, name: &str) -> WorkflowResult<(String, u64)> {
    let manifest = manifest_path(&paths.workflows, name);
    if !manifest.exists() {
        return Err(WorkflowError::NotFound { name: name.to_string() });
    }
    let contents = std::fs::read_to_string(&manifest).map_err(WorkflowError::Io)?;
    let modified = modified_at(&manifest)?;
    Ok((contents, modified))
}

/// Explicitly save a workflow definition.
///
/// - Validates the YAML against the P17 schema before writing (a broken save
///   never lands on disk).
/// - If `expected_mtime` is `Some` and differs from the current formal file's
///   mtime, returns [`WorkflowError::Conflict`] (P27 冲突检测).
/// - Writes atomically, then removes any stale draft.
///
/// Returns the new mtime (seconds) of the formal file.
pub fn save_workflow(
    paths: &CaspianPaths,
    name: &str,
    yaml: &str,
    expected_mtime: Option<u64>,
) -> WorkflowResult<u64> {
    // Validate first — never persist an unparseable manifest.
    Workflow::from_yaml_at(yaml, &manifest_path(&paths.workflows, name))
        .map_err(|e| WorkflowError::ParseError {
            path: manifest_path(&paths.workflows, name)
                .display()
                .to_string(),
            reason: e.to_string(),
        })?;

    let wf_dir = ensure_dirs(paths, name)?;
    let manifest = wf_dir.join("workflow.yaml");

    // Conflict detection: compare against the recorded load-time mtime.
    if let Some(expected) = expected_mtime {
        let current = if manifest.exists() {
            modified_at(&manifest)?
        } else {
            0
        };
        if current != expected {
            return Err(WorkflowError::Conflict {
                name: name.to_string(),
                reason: format!(
                    "formal file mtime changed ({current} != {expected}); external edit detected"
                ),
            });
        }
    }

    atomic_write(&manifest, yaml)?;
    let new_mtime = modified_at(&manifest)?;

    // Explicit save supersedes the draft — clean it up (验收 #4).
    let dp = draft_path(&paths.workflows, name);
    if dp.exists() {
        let _ = std::fs::remove_file(&dp);
    }

    Ok(new_mtime)
}

/// Write (or overwrite) a draft. Drafts are unobserved by the engine scanner,
/// so a partial/crashed draft can never pollute the executable set.
pub fn save_draft(paths: &CaspianPaths, name: &str, yaml: &str) -> WorkflowResult<()> {
    let drafts = paths.workflows.join(".drafts");
    std::fs::create_dir_all(&drafts).map_err(WorkflowError::Io)?;
    let dp = drafts.join(format!("{name}.yaml"));
    atomic_write(&dp, yaml)
}

/// Whether a draft exists for the named workflow.
pub fn has_draft(paths: &CaspianPaths, name: &str) -> bool {
    draft_path(&paths.workflows, name).exists()
}

/// Delete a workflow definition (its directory) and any stale draft.
pub fn delete_workflow(paths: &CaspianPaths, name: &str) -> WorkflowResult<()> {
    let wf_dir = paths.workflows.join(name);
    if !wf_dir.exists() {
        return Err(WorkflowError::NotFound { name: name.to_string() });
    }
    std::fs::remove_dir_all(&wf_dir).map_err(WorkflowError::Io)?;
    let dp = draft_path(&paths.workflows, name);
    if dp.exists() {
        let _ = std::fs::remove_file(&dp);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const WF_YAML: &str = r#"schema_version: "1.0"
name: "process_document"
display_name: "Process Document"
version: "1.0.0"
description: "Read, summarize, and save a document"
steps:
  - id: "read"
    skill: "read_file"
  - id: "summarize"
    skill: "summarize_text"
    depends_on: ["read"]
"#;

    fn paths() -> (tempfile::TempDir, CaspianPaths) {
        let dir = tempfile::tempdir().unwrap();
        let p = CaspianPaths::resolve(Some(dir.path()));
        (dir, p)
    }

    #[test]
    fn test_save_then_read_roundtrip() {
        let (_d, p) = paths();
        let mtime = save_workflow(&p, "process_document", WF_YAML, None).unwrap();
        assert!(mtime > 0);

        let (wf, loaded_mtime) = read_workflow(&p, "process_document").unwrap();
        assert_eq!(wf.name, "process_document");
        assert_eq!(wf.step_count(), 2);
        assert_eq!(loaded_mtime, mtime);

        // Manifest lives at the P17 subdirectory path.
        assert!(manifest_path(&p.workflows, "process_document").exists());
        assert!(p
            .workflows
            .join("process_document")
            .join("workflow.yaml")
            .exists());
    }

    #[test]
    fn test_save_rejects_invalid_yaml() {
        let (_d, p) = paths();
        let bad = "name: : : oops";
        let err = save_workflow(&p, "broken", bad, None);
        assert!(err.is_err());
        // Nothing was written.
        assert!(!p.workflows.join("broken").exists());
    }

    #[test]
    fn test_atomic_save_no_partial_on_crash() {
        // Simulate by writing a valid manifest, confirming it's complete & single.
        let (_d, p) = paths();
        save_workflow(&p, "w", WF_YAML, None).unwrap();
        let path = manifest_path(&p.workflows, "w");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("summarize_text"));
        // No leftover temp file.
        let tmp = path.parent().unwrap().join(".workflow.yaml.tmp");
        assert!(!tmp.exists());
    }

    #[test]
    fn test_draft_isolated_from_list() {
        let (_d, p) = paths();
        save_workflow(&p, "real", WF_YAML, None).unwrap();
        save_draft(&p, "real", WF_YAML).unwrap();

        assert!(has_draft(&p, "real"));
        // The draft file exists but the scanner/list never surfaces it.
        let entries = list_entries(&p).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "real");
        // Draft dir itself not enumerated as a workflow.
        assert!(p.workflows.join(".drafts").join("real.yaml").exists());
    }

    #[test]
    fn test_explicit_save_clears_draft() {
        let (_d, p) = paths();
        save_workflow(&p, "w", WF_YAML, None).unwrap();
        save_draft(&p, "w", WF_YAML).unwrap();
        assert!(has_draft(&p, "w"));
        save_workflow(&p, "w", WF_YAML, None).unwrap();
        assert!(!has_draft(&p, "w"), "explicit save must clean the draft");
    }

    #[test]
    fn test_conflict_when_external_edit_changes_mtime() {
        let (_d, p) = paths();
        let mtime = save_workflow(&p, "w", WF_YAML, None).unwrap();

        // External editor touches the formal file (changes mtime).
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let manifest = manifest_path(&p.workflows, "w");
        std::fs::write(&manifest, WF_YAML).unwrap();
        let new_mtime = modified_at(&manifest).unwrap();
        assert_ne!(new_mtime, mtime, "external edit must shift mtime");

        // Save carrying the stale (recorded) mtime → Conflict.
        let err = save_workflow(&p, "w", WF_YAML, Some(mtime));
        assert!(matches!(err, Err(WorkflowError::Conflict { .. })));

        // Save without expected_mtime (or with the fresh one) succeeds.
        let ok = save_workflow(&p, "w", WF_YAML, None).unwrap();
        assert!(ok >= new_mtime);
    }

    #[test]
    fn test_conflict_none_for_new_workflow() {
        let (_d, p) = paths();
        // New workflow: no file yet, expected_mtime = 0 → no conflict.
        let mtime = save_workflow(&p, "fresh", WF_YAML, Some(0)).unwrap();
        assert!(mtime > 0);
    }

    #[test]
    fn test_delete_removes_dir_and_draft() {
        let (_d, p) = paths();
        save_workflow(&p, "w", WF_YAML, None).unwrap();
        save_draft(&p, "w", WF_YAML).unwrap();
        delete_workflow(&p, "w").unwrap();
        assert!(!p.workflows.join("w").exists());
        assert!(!has_draft(&p, "w"));

        // Deleting a missing workflow errors.
        assert!(matches!(
            delete_workflow(&p, "w"),
            Err(WorkflowError::NotFound { .. })
        ));
    }

    #[test]
    fn test_list_entries_shape() {
        let (_d, p) = paths();
        save_workflow(&p, "alpha", WF_YAML, None).unwrap();
        let entries = list_entries(&p).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display_name, "Process Document");
        assert_eq!(entries[0].step_count, 2);
    }
}
