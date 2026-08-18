//! Python runtime adapter.
//!
//! Executes `.py` scripts using `python3`.

use std::process::Stdio;

use crate::skill::schema::{Skill, SkillRuntimeType};
use crate::types::{ExecutorError, ExecutorResult};

use super::adapter::RuntimeAdapter;

/// Adapter for executing Python skills.
///
/// Uses `python3` as the interpreter. The entry file (e.g. `script.py`)
/// is passed as the sole argument, and input JSON is written to stdin.
pub struct PythonRuntimeAdapter;

#[async_trait::async_trait]
impl RuntimeAdapter for PythonRuntimeAdapter {
    async fn check_available(&self) -> ExecutorResult<()> {
        match tokio::process::Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
        {
            Ok(status) if status.success() => Ok(()),
            Ok(_) => Err(ExecutorError::RuntimeNotFound {
                runtime: SkillRuntimeType::Python.to_string(),
                reason: "python3 exited with non-zero status".to_string(),
            }),
            Err(e) => Err(ExecutorError::RuntimeNotFound {
                runtime: SkillRuntimeType::Python.to_string(),
                reason: format!("python3 not found: {e}"),
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
            "python3".to_string(),
            vec![entry_path.to_string_lossy().to_string()],
        ))
    }

    fn runtime_type(&self) -> SkillRuntimeType {
        SkillRuntimeType::Python
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
            name: "test_python".to_string(),
            display_name: "Test Python".to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            category: "test".to_string(),
            trigger_phrases: vec![],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Python,
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
        let adapter = PythonRuntimeAdapter;
        let skill = make_skill("nonexistent.py", "/tmp");
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
        let script_path = tmp.path().join("script.py");
        std::fs::write(&script_path, "print('hello')").unwrap();

        let skill = make_skill("script.py", tmp.path().to_str().unwrap());
        let adapter = PythonRuntimeAdapter;
        let (exec, args) = adapter.build_args(&skill).unwrap();
        assert_eq!(exec, "python3");
        assert_eq!(args.len(), 1);
        assert!(args[0].ends_with("script.py"));
    }

    #[test]
    fn test_runtime_type() {
        let adapter = PythonRuntimeAdapter;
        assert_eq!(adapter.runtime_type(), SkillRuntimeType::Python);
    }
}
