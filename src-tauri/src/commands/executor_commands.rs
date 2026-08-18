//! Executor IPC commands.
//!
//! These functions are designed to be annotated with `#[tauri::command]`
//! once the Tauri runtime is integrated. For now they are plain async
//! functions that can be called from Rust or wrapped by the frontend bridge.

use serde_json::Value;

use crate::skill::executor::{ExecutionConfig, ExecutionResult, Executor};
use crate::skill::schema::Skill;
use crate::types::ExecutorResult;

/// Create a new executor with the given configuration.
pub fn create_executor(config: ExecutionConfig) -> Executor {
    Executor::new(config)
}

/// Create an executor with default configuration.
pub fn create_default_executor() -> Executor {
    Executor::with_defaults()
}

/// Execute a skill with the given input.
///
/// This is the main entry point for skill execution via IPC.
pub async fn execute_skill(
    executor: &Executor,
    skill: &Skill,
    input: &Value,
) -> ExecutorResult<ExecutionResult> {
    executor.execute(skill, input).await
}

/// Check if an execution result was successful.
pub fn is_success(result: &ExecutionResult) -> bool {
    result.success
}

/// Get the stdout from an execution result.
pub fn get_stdout(result: &ExecutionResult) -> &str {
    &result.stdout
}

/// Get the stderr from an execution result.
pub fn get_stderr(result: &ExecutionResult) -> &str {
    &result.stderr
}

/// Get the exit code from an execution result.
pub fn get_exit_code(result: &ExecutionResult) -> Option<i32> {
    result.exit_code
}

/// Get the execution duration in milliseconds.
pub fn get_duration_ms(result: &ExecutionResult) -> u64 {
    result.duration_ms
}

/// Check if the execution timed out.
pub fn was_timed_out(result: &ExecutionResult) -> bool {
    result.timed_out
}

/// Get the executor configuration.
pub fn get_config(executor: &Executor) -> &ExecutionConfig {
    executor.config()
}

/// Create a default ExecutionConfig.
pub fn default_config() -> ExecutionConfig {
    ExecutionConfig::default()
}

/// Create a custom ExecutionConfig.
pub fn custom_config(
    max_concurrent: usize,
    default_timeout: u64,
    default_memory_limit_mb: u32,
    capture_stderr: bool,
) -> ExecutionConfig {
    ExecutionConfig {
        max_concurrent,
        default_timeout,
        default_memory_limit_mb,
        capture_stderr,
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

    fn make_shell_skill(name: &str, entry: &str, path: &str) -> Skill {
        Skill {
            schema_version: "1.0".to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Test skill {name}"),
            category: "test".to_string(),
            trigger_phrases: vec!["test".to_string()],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Shell,
                entry: entry.to_string(),
                timeout: 30,
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
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from(path),
            mcp: None,
        }
    }

    #[test]
    fn test_create_default_executor() {
        let executor = create_default_executor();
        let config = get_config(&executor);
        assert_eq!(config.max_concurrent, 4);
        assert_eq!(config.default_timeout, 60);
    }

    #[test]
    fn test_create_executor_with_config() {
        let config = custom_config(8, 120, 512, false);
        let executor = create_executor(config);
        let config = get_config(&executor);
        assert_eq!(config.max_concurrent, 8);
        assert_eq!(config.default_timeout, 120);
        assert_eq!(config.default_memory_limit_mb, 512);
        assert!(!config.capture_stderr);
    }

    #[test]
    fn test_default_config_command() {
        let config = default_config();
        assert_eq!(config.max_concurrent, 4);
        assert_eq!(config.default_timeout, 60);
        assert_eq!(config.default_memory_limit_mb, 256);
        assert!(config.capture_stderr);
    }

    #[tokio::test]
    async fn test_execute_skill_command_success() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\ncat\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        let skill = make_shell_skill("echo", "run.sh", tmp.path().to_str().unwrap());
        let executor = create_default_executor();
        let input = serde_json::json!({"msg": "hello"});

        let result = execute_skill(&executor, &skill, &input).await.unwrap();

        assert!(is_success(&result));
        assert!(!was_timed_out(&result));
        assert_eq!(get_exit_code(&result), Some(0));
        assert!(get_stdout(&result).contains("hello"));
        assert!(get_duration_ms(&result) < 5000);
    }

    #[tokio::test]
    async fn test_execute_skill_command_non_zero_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\nexit 1\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        let skill = make_shell_skill("fail", "run.sh", tmp.path().to_str().unwrap());
        let executor = create_default_executor();
        let input = serde_json::json!({});

        let result = execute_skill(&executor, &skill, &input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_skill_command_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\nsleep 30\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        let mut skill = make_shell_skill("slow", "run.sh", tmp.path().to_str().unwrap());
        skill.runtime.timeout = 1;

        let executor = create_default_executor();
        let input = serde_json::json!({});

        let result = execute_skill(&executor, &skill, &input).await;
        assert!(result.is_err());
    }
}
