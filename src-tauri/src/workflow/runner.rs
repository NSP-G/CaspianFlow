//! P28 run-path closure (sandbox-runnable, **not** Tauri-gated).
//!
//! Validates the exact path `run_workflow` uses: derive the manifest path from
//! `CaspianPaths::workflows`, `Workflow::load` it, build a `WorkflowEngine` from
//! a disk-populated `SkillRegistry` + `RunStore`, `execute`, and confirm the run
//! is persisted as `Completed` with per-step output files on disk.
//!
//! This is the "serialize → engine load → execute → RunStore persist" chain with
//! no Tauri runtime (webkit2gtk is absent in CI/sandbox), satisfying the P28
//! sandbox strategy (设计文档 §六). The real Tauri command (`tauri_app.rs`,
//! feature-gated) reuses this exact formula.

#[cfg(test)]
mod tests {
  use std::collections::HashMap;
  use std::sync::Arc;

  use crate::config::CaspianPaths;
  use crate::skill::SkillManager;
  use crate::workflow::store::{RunStatus, RunStore};
  use crate::workflow::{Workflow, WorkflowEngine};

  /// Install a minimal, self-contained shell skill on disk so the engine has
  /// something executable without external runtimes.
  fn make_shell_skill(skills_dir: &std::path::Path, name: &str) {
    let dir = skills_dir.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("run.sh");
    std::fs::write(&script, "#!/bin/sh\necho '{\"v\": 1}'\n").unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
    }
    let yaml = format!(
      r#"schema_version: "1.0"
name: "{name}"
display_name: "{name}"
version: "1.0.0"
description: "test shell skill"
category: "test"
trigger_phrases:
  - "test"
runtime:
  type: "shell"
  entry: "run.sh"
  timeout: 30
  memory_limit_mb: 256
input_schema:
  type: "object"
output_schema:
  type: "object"
permissions:
  fs: []
  network: false
  shell: true
tags:
  - "test"
author: "test"
license: "MIT"
"#
    );
    std::fs::write(dir.join("skill.yaml"), yaml).unwrap();
  }

  const DEMO_WORKFLOW: &str = r#"schema_version: "1.0"
name: "demo"
display_name: "Demo"
version: "1.0.0"
description: "demo workflow"
steps:
  - id: "step1"
    skill: "echo_ok"
    input: {}
"#;

  #[tokio::test]
  async fn run_path_executes_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let paths = CaspianPaths::resolve(Some(dir.path()));
    paths.ensure_dirs().unwrap();

    // A real shell skill on disk, so the engine has something to execute.
    make_shell_skill(&paths.skills, "echo_ok");

    let manager = SkillManager::init(&paths.skills).await.unwrap();
    assert!(
      manager.registry().exists("echo_ok"),
      "shell skill must be registered after scan"
    );

    let store = Arc::new(RunStore::from_paths(&paths));

    // Write the workflow definition under the P27/P17 subdir convention.
    let name = "demo";
    let wf_dir = paths.workflows.join(name);
    std::fs::create_dir_all(&wf_dir).unwrap();
    let manifest = wf_dir.join("workflow.yaml");
    std::fs::write(&manifest, DEMO_WORKFLOW).unwrap();

    // --- the exact path `run_workflow` takes ---
    let workflow = Workflow::load(&manifest).unwrap();
    assert_eq!(workflow.path, wf_dir);

    let engine = WorkflowEngine::with_defaults(manager.shared_registry(), Arc::clone(&store));
    let result = engine
      .execute(&workflow, &HashMap::new())
      .await
      .expect("workflow should execute end-to-end");

    assert_eq!(result.steps.len(), 1);
    assert_eq!(result.steps[0].step_id, "step1");

    // RunStore persistence — the sandbox equivalent of 验收 #7 (运行可查).
    let runs = store.list_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Completed);
    assert!(
      runs[0].steps[0].output_path.exists(),
      "per-step output must be persisted"
    );
  }

  #[test]
  fn run_path_manifest_location_matches_command() {
    // Documents the precise path formula `run_workflow` uses, independent of
    // execution (no Tauri / no subprocess needed).
    let dir = tempfile::tempdir().unwrap();
    let paths = CaspianPaths::resolve(Some(dir.path()));
    paths.ensure_dirs().unwrap();

    let name = "demo";
    let manifest = paths.workflows.join(name).join("workflow.yaml");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(
      &manifest,
      r#"schema_version: "1.0"
name: "demo"
steps:
  - id: "only"
    skill: "echo_ok"
    input: {}
"#,
    )
    .unwrap();

    let wf = Workflow::load(&manifest).unwrap();
    assert_eq!(wf.name, "demo");
    assert_eq!(wf.path, paths.workflows.join(name));
  }
}
