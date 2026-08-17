//! Workflow discovery — walks `~/.caspian/workflows/` and enumerates every
//! user-authored workflow definition.
//!
//! ## P27 (模式 C) contract
//!
//! Drafts live under `~/.caspian/workflows/.drafts/` and are **never**
//! enumerated here — a workflow canvas may crash or be half-written without
//! ever polluting the executable set the P17 engine reads. Any directory whose
//! name starts with `.` is skipped for exactly this reason, so `.drafts` is
//! excluded by construction (acceptance #6).
//!
//! Each workflow lives in its own subdirectory per the P17 convention
//! (`schema.rs`): `<workflows>/<name>/workflow.yaml`. The manifest's parent
//! directory is the workflow's `path` (see `Workflow::load`).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::config::CaspianPaths;
use crate::types::{WorkflowError, WorkflowResult};

use super::schema::Workflow;

/// Discovers workflow definitions under a root directory.
///
/// Construct from [`CaspianPaths::workflows`] or any temp root in tests.
#[derive(Debug, Clone)]
pub struct WorkflowScanner {
    root: PathBuf,
}

/// A discovered workflow plus the last-modified time of its `workflow.yaml`.
///
/// `modified` is the unix timestamp (seconds) used by the canvas for mtime
/// conflict detection (P27 验收 #5). `dir_name` is the **filesystem identity**
/// of the workflow (the `<name>` directory) and is the key used by all
/// save/load/delete operations — distinct from `workflow.name`, which is the
/// human-facing label stored inside the YAML.
#[derive(Debug, Clone)]
pub struct WorkflowSummary {
    /// The parsed workflow (its `path` field points at the `<name>` directory).
    pub workflow: Workflow,
    /// mtime of `workflow.yaml` in seconds since the unix epoch.
    pub modified: u64,
    /// The directory basename — the canonical workflow id used by the canvas.
    pub dir_name: String,
}

impl WorkflowScanner {
    /// Create a scanner rooted at `root` (`~/.caspian/workflows`).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Create a scanner from the resolved CaspianFlow paths.
    pub fn from_paths(paths: &CaspianPaths) -> Self {
        Self::new(paths.workflows.clone())
    }

    /// Root directory being scanned.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return every valid workflow under `root`.
    ///
    /// Hidden directories (names starting with `.`) are skipped — this is what
    /// keeps `.drafts/` out of the executable set. Subdirectories without a
    /// `workflow.yaml` are ignored. A workflow whose `workflow.yaml` fails to
    /// parse is skipped (the P17 engine re-validates on execution; discovery
    /// must not hard-fail the whole scan).
    ///
    /// Results are ordered by `modified` descending (newest first).
    pub fn list(&self) -> WorkflowResult<Vec<WorkflowSummary>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut summaries: Vec<WorkflowSummary> = Vec::new();
        let mut entries = std::fs::read_dir(&self.root).map_err(|e| {
            WorkflowError::ScanError(format!("read_dir {}: {e}", self.root.display()))
        })?;

        while let Some(entry) = entries.next().transpose().map_err(|e| {
            WorkflowError::ScanError(format!("read_dir {}: {e}", self.root.display()))
        })? {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // Skip hidden dirs — `.drafts` and any future dot-dir.
            if dir_name.starts_with('.') {
                continue;
            }

            let manifest = path.join("workflow.yaml");
            if !manifest.exists() {
                continue;
            }

            let workflow = match Workflow::load(&manifest) {
                Ok(w) => w,
                Err(e) => {
                    // Skip unparseable manifests rather than failing the scan.
                    eprintln!(
                        "workflow scanner: skipping {} (parse error: {e})",
                        manifest.display()
                    );
                    continue;
                }
            };
            let modified = modified_at(&manifest).unwrap_or(0);
            summaries.push(WorkflowSummary {
                workflow,
                modified,
                dir_name: dir_name.to_string(),
            });
        }

        summaries.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(summaries)
    }

    /// Load a single workflow by its directory name (the `<name>` segment).
    ///
    /// Returns [`WorkflowError::NotFound`] when the manifest is absent.
    pub fn load_by_name(&self, name: &str) -> WorkflowResult<Workflow> {
        let manifest = self.root.join(name).join("workflow.yaml");
        if !manifest.exists() {
            return Err(WorkflowError::NotFound { name: name.to_string() });
        }
        Workflow::load(&manifest)
    }
}

/// Unix mtime (seconds) of a file, or an error if it cannot be read.
pub fn modified_at(path: &Path) -> WorkflowResult<u64> {
    let meta = std::fs::metadata(path).map_err(|e| {
        WorkflowError::ScanError(format!("metadata {}: {e}", path.display()))
    })?;
    let modified = meta.modified().map_err(|e| {
        WorkflowError::ScanError(format!("modified {}: {e}", path.display()))
    })?;
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0))
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

    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workflows");
        std::fs::create_dir_all(&root).unwrap();
        (dir, root)
    }

    #[test]
    fn test_list_finds_workflow_subdir() {
        let (_d, root) = scratch();
        let wf_dir = root.join("process_document");
        std::fs::create_dir_all(&wf_dir).unwrap();
        std::fs::write(wf_dir.join("workflow.yaml"), WF_YAML).unwrap();

        let scanner = WorkflowScanner::new(root);
        let list = scanner.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].workflow.name, "process_document");
        assert_eq!(list[0].workflow.path, wf_dir);
        assert!(list[0].modified > 0);
    }

    #[test]
    fn test_list_skips_drafts_dir() {
        let (_d, root) = scratch();
        // A "real" workflow.
        let wf_dir = root.join("real_one");
        std::fs::create_dir_all(&wf_dir).unwrap();
        std::fs::write(wf_dir.join("workflow.yaml"), WF_YAML).unwrap();

        // A draft under .drafts/ — must NOT be enumerated (acceptance #6).
        let draft_dir = root.join(".drafts");
        std::fs::create_dir_all(&draft_dir).unwrap();
        std::fs::write(draft_dir.join("real_one.yaml"), WF_YAML).unwrap();
        // Also a half-written hidden dir with a valid-looking manifest.
        let hidden = root.join(".tmp");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("workflow.yaml"), WF_YAML).unwrap();

        let scanner = WorkflowScanner::new(root);
        let list = scanner.list().unwrap();
        assert_eq!(list.len(), 1, "drafts + hidden dirs must be skipped");
        assert_eq!(list[0].dir_name, "real_one");
    }

    #[test]
    fn test_list_ignores_dir_without_manifest() {
        let (_d, root) = scratch();
        std::fs::create_dir_all(root.join("no_manifest")).unwrap();
        std::fs::write(root.join("no_manifest").join("readme.txt"), "x").unwrap();

        let scanner = WorkflowScanner::new(root);
        assert!(scanner.list().unwrap().is_empty());
    }

    #[test]
    fn test_list_empty_when_root_missing() {
        let dir = tempfile::tempdir().unwrap();
        let scanner = WorkflowScanner::new(dir.path().join("does_not_exist"));
        assert!(scanner.list().unwrap().is_empty());
    }

    #[test]
    fn test_list_skips_unparseable_manifest() {
        let (_d, root) = scratch();
        let bad = root.join("broken");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("workflow.yaml"), "name: : : not yaml : :").unwrap();

        let good = root.join("good");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(good.join("workflow.yaml"), WF_YAML).unwrap();

        let scanner = WorkflowScanner::new(root);
        let list = scanner.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].dir_name, "good");
    }

    #[test]
    fn test_load_by_name() {
        let (_d, root) = scratch();
        let wf_dir = root.join("process_document");
        std::fs::create_dir_all(&wf_dir).unwrap();
        std::fs::write(wf_dir.join("workflow.yaml"), WF_YAML).unwrap();

        let scanner = WorkflowScanner::new(root);
        let wf = scanner.load_by_name("process_document").unwrap();
        assert_eq!(wf.name, "process_document");
        assert_eq!(wf.step_count(), 2);
    }

    #[test]
    fn test_load_by_name_not_found() {
        let (_d, root) = scratch();
        let scanner = WorkflowScanner::new(root);
        assert!(matches!(
            scanner.load_by_name("ghost"),
            Err(WorkflowError::NotFound { .. })
        ));
    }

    #[test]
    fn test_modified_at_returns_positive() {
        let (_d, root) = scratch();
        let f = root.join("a.yaml");
        std::fs::write(&f, "x").unwrap();
        assert!(modified_at(&f).unwrap() > 0);
    }
}
