//! Shell runtime adapter.
//!
//! Executes `.sh` scripts using `sh`.
//!
//! ## Memory limit note
//!
//! Shell runtimes skip the `ulimit` memory-limiting wrapper to avoid
//! nested shell layers. The design decision is documented in
//! [`Executor`](super::Executor).

use std::process::Stdio;

use crate::skill::schema::{Skill, SkillRuntimeType};
use crate::types::{ExecutorError, ExecutorResult};

use super::adapter::RuntimeAdapter;

/// Adapter for executing Shell skills.
///
/// Uses `sh` as the interpreter. The entry file (e.g. `run.sh`)
/// is passed as the sole argument, and input JSON is written to stdin.
pub struct ShellRuntimeAdapter;

#[async_trait::async_trait]
impl RuntimeAdapter for ShellRuntimeAdapter {
    async fn check_available(&self) -> ExecutorResult<()> {
        match tokio::process::Command::new("sh")
            .arg("-c")
            .arg("echo ok")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
        {
            Ok(status) if status.success() => Ok(()),
            Ok(_) => Err(ExecutorError::RuntimeNotFound {
                runtime: SkillRuntimeType::Shell.to_string(),
                reason: "sh exited with non-zero status".to_string(),
            }),
            Err(e) => Err(ExecutorError::RuntimeNotFound {
                runtime: SkillRuntimeType::Shell.to_string(),
                reason: format!("sh not found: {e}"),
            }),
        }
    }

    fn build_args(&self, skill: &Skill) -> ExecutorResult<(String, Vec<String>)> {
        let entry_path = skill.entry_path();
        if !entry_path.exists() {
            return Err(ExecutorError::EntryNotFound {
                path: entry_path.display().to_string(),
            });
        }
        Ok((
            "sh".to_string(),
            vec![entry_path.to_string_lossy().to_string()],
        ))
    }

    fn runtime_type(&self) -> SkillRuntimeType {
        SkillRuntimeType::Shell
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::{SkillRuntime, SkillRuntimeType};
    use std::path::PathBuf;

    fn make_skill(entry: &str, path: &str) -> Skill {
        Skill {
            mcp: None,
            schema_version: "1.0".to_string(),
            name: "test_shell".to_string(),
            display_name: "Test Shell".to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            category: "test".to_string(),
            trigger_phrases: vec![],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Shell,
                entry: entry.to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            permissions: Default::default(),
            tags: vec![],
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn test_build_args_entry_not_found() {
        let adapter = ShellRuntimeAdapter;
        let skill = make_skill("nonexistent.sh", "/tmp");
        let result = adapter.build_args(&skill);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ExecutorError::EntryNotFound { .. }
        ));
    }

    #[test]
    fn test_build_args_success() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("run.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho hello\n").unwrap();

        let skill = make_skill("run.sh", tmp.path().to_str().unwrap());
        let adapter = ShellRuntimeAdapter;
        let (exec, args) = adapter.build_args(&skill).unwrap();
        assert_eq!(exec, "sh");
        assert_eq!(args.len(), 1);
        assert!(args[0].ends_with("run.sh"));
    }

    #[test]
    fn test_runtime_type() {
        let adapter = ShellRuntimeAdapter;
        assert_eq!(adapter.runtime_type(), SkillRuntimeType::Shell);
    }
}
