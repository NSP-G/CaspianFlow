//! Skill execution engine — manages subprocess execution for all runtime types.
//!
//! ## Overview
//!
//! The executor takes a [`Skill`] and its input parameters, spawns a subprocess
//! using the appropriate runtime adapter (Python, JavaScript, RustBinary, Shell),
//! and returns the standardized output.
//!
//! ## Execution flow
//!
//! ```text
//! SlotFiller output (JSON)
//!     │
//!     v
//! Executor::execute(skill, input)
//!     │
//!     ├─ Select RuntimeAdapter based on skill.runtime.runtime_type
//!     ├─ Check runtime availability (python3/node/etc.)
//!     ├─ Build command (executable + args)
//!     ├─ Apply memory limit (ulimit wrapper, non-Shell only)
//!     ├─ Set stdin = JSON input, stdout/stderr = piped
//!     ├─ Acquire pool permit (concurrency control)
//!     ├─ Spawn process + write stdin
//!     ├─ Wait with timeout (kill on timeout)
//!     └─ Return ExecutionResult
//!     │
//!     v
//! Guardian::validate_with_retry(skill, result.stdout)
//! ```
//!
//! ## Memory limiting
//!
//! For non-Shell runtimes, the command is wrapped in:
//! `sh -c "ulimit -v {memory_kb}; exec {original_command}"`
//!
//! Shell runtimes skip this wrapping to avoid nested shell layers that could
//! affect script behavior.
//!
//! **Known limitation**: `ulimit -v` limits *virtual* memory, not physical
//! memory. V8-based runtimes (Node.js) reserve large amounts of virtual
//! memory for their heap and may fail with OOM at low limits. For
//! JavaScript skills, either set a higher limit (≥ 1GB) or disable
//! memory limiting (`default_memory_limit_mb: 0`).

pub mod adapter;
pub mod javascript;
pub mod pool;
pub mod sandbox;
pub mod python;
pub mod rust_binary;
pub mod shell;

pub use adapter::{create_adapter, RuntimeAdapter};
pub use pool::ExecutionPool;

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::skill::schema::{McpRef, Skill, SkillRuntimeType};
use crate::types::{ExecutorError, ExecutorResult};

use sandbox::{apply_sandbox_env, check_runtime_permissions, SkillSandbox};

/// Default maximum concurrent executions.
const DEFAULT_MAX_CONCURRENT: usize = 4;

/// Default timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Default memory limit in MB.
const DEFAULT_MEMORY_LIMIT_MB: u32 = 256;

/// Configuration for the skill executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Maximum number of concurrent skill executions (default: 4).
    pub max_concurrent: usize,
    /// Default timeout in seconds when skill doesn't specify one (default: 60).
    pub default_timeout: u64,
    /// Default memory limit in MB (default: 256).
    pub default_memory_limit_mb: u32,
    /// Whether to capture stderr output (default: true).
    pub capture_stderr: bool,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            default_timeout: DEFAULT_TIMEOUT_SECS,
            default_memory_limit_mb: DEFAULT_MEMORY_LIMIT_MB,
            capture_stderr: true,
        }
    }
}

/// The result of a skill execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The stdout output from the process.
    pub stdout: String,
    /// The stderr output from the process.
    pub stderr: String,
    /// The exit code of the process (None if terminated by signal).
    pub exit_code: Option<i32>,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the execution timed out.
    pub timed_out: bool,
    /// Whether the execution was successful (exit code 0).
    pub success: bool,
}

/// The skill executor — manages subprocess execution with timeout and resource limits.
///
/// ## Usage
///
/// ```no_run
/// # use caspian_flow::skill::executor::{Executor, ExecutionConfig};
/// # use caspian_flow::skill::schema::Skill;
/// # use serde_json::json;
/// # async fn example(skill: &Skill) {
/// let executor = Executor::with_defaults();
/// let result = executor.execute(skill, &json!({"path": "/tmp"})).await.unwrap();
/// println!("stdout: {}", result.stdout);
/// # }
/// ```
pub struct Executor {
    config: ExecutionConfig,
    pool: ExecutionPool,
}

impl Executor {
    /// Create a new executor with the given configuration.
    pub fn new(config: ExecutionConfig) -> Self {
        let pool = ExecutionPool::new(config.max_concurrent);
        Self { config, pool }
    }

    /// Create an executor with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(ExecutionConfig::default())
    }

    /// Get the executor configuration.
    pub fn config(&self) -> &ExecutionConfig {
        &self.config
    }

    /// Execute a skill with the given input.
    ///
    /// This is the main entry point for skill execution. It:
    /// 1. Selects the appropriate runtime adapter
    /// 2. Checks runtime availability
    /// 3. Builds and spawns the subprocess
    /// 4. Writes input JSON to stdin
    /// 5. Waits for completion with timeout
    /// 6. Returns the standardized output
    pub async fn execute(&self, skill: &Skill, input: &Value) -> ExecutorResult<ExecutionResult> {
        let start = Instant::now();

        // 0a. Enforce declared permissions (P32 安全沙箱). The shell gate is
        // fully enforceable in-process; other policies are recorded for a future
        // OS-level harness (see `sandbox` module docs).
        check_runtime_permissions(skill)?;

        // 0a'. MCP-backed skills bypass the entire local subprocess adapter:
        // instead of spawning a script, we route the call through an external
        // MCP server over JSON-RPC 2.0 stdio (B-2 检查点: 最小客户端, 复用 A4
        // 沙箱运行外部代码)。This keeps the core executor free of heavy MCP SDK
        // dependencies while still exposing arbitrary MCP tools as skills.
        if let Some(mcp) = &skill.mcp {
            return self.execute_mcp_skill(skill, mcp, input, start).await;
        }

        // 0b. Create a private, disposable sandbox directory for this execution.
        // `sandbox` is held until `execute()` returns — on either the success or
        // error path — so the directory is always cleaned up afterwards.
        let sandbox = SkillSandbox::new()?;

        // 1. Create adapter
        let adapter = create_adapter(skill.runtime.runtime_type.clone());

        // 2. Check availability
        adapter.check_available().await?;

        // 3. Build args
        let (exec, args) = adapter.build_args(skill)?;

        // 4. Apply memory limit (non-Shell only — Shell skips to avoid nesting)
        let (exec, args) = if skill.runtime.runtime_type != SkillRuntimeType::Shell
            && self.config.default_memory_limit_mb > 0
        {
            wrap_with_ulimit(&exec, &args, self.config.default_memory_limit_mb)
        } else {
            (exec, args)
        };

        // 5. Serialize input
        let input_json = serde_json::to_string(input)
            .map_err(|e| ExecutorError::InputSerialization(e.to_string()))?;

        // 6. Determine timeout (skill-specific overrides config default)
        let timeout_secs = if skill.runtime.timeout > 0 {
            skill.runtime.timeout
        } else {
            self.config.default_timeout
        };

        // 7. Build command
        let mut cmd = Command::new(&exec);
        cmd.args(&args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        if self.config.capture_stderr {
            cmd.stderr(Stdio::piped());
        } else {
            cmd.stderr(Stdio::null());
        }
        cmd.env("CASPIAN_SKILL_NAME", &skill.name);
        cmd.env(
            "CASPIAN_WORKSPACE",
            skill.dir().to_string_lossy().to_string(),
        );
        // Apply P32 sandbox isolation + declared permission markers (sets the
        // sandbox CWD and the CASPIAN_* policy env vars).
        apply_sandbox_env(&mut cmd, &sandbox, skill);

        tracing::info!(
            skill = %skill.name,
            runtime = %skill.runtime.runtime_type,
            timeout_secs,
            "starting skill execution"
        );

        // 8. Acquire pool permit
        let _permit = self.pool.acquire().await?;

        // 9. Spawn process
        let mut child = cmd.spawn()?;

        // 10. Write input to stdin.
        //
        // A skill is free to ignore stdin entirely (e.g. a script that only
        // echoes, or one driven purely by argv). Such a process may exit before
        // we finish writing, which surfaces as `BrokenPipe`. That is not an
        // execution failure — the verdict belongs to the exit status below — so
        // a broken pipe is swallowed while real IO errors still propagate.
        if let Some(mut stdin) = child.stdin.take() {
            match stdin.write_all(input_json.as_bytes()).await {
                Ok(()) => {
                    if let Err(e) = stdin.shutdown().await {
                        if e.kind() != std::io::ErrorKind::BrokenPipe {
                            return Err(e.into());
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(e) => return Err(e.into()),
            }
        }

        // 11. Take stdout/stderr handles before waiting
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // 12. Wait with timeout
        let wait_result =
            tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match wait_result {
            // Timeout
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await; // Reap zombie
                tracing::warn!(
                    skill = %skill.name,
                    timeout_secs,
                    "skill execution timed out"
                );
                Err(ExecutorError::Timeout {
                    skill_name: skill.name.clone(),
                    timeout_secs,
                })
            }
            // IO error during wait
            Ok(Err(e)) => {
                tracing::error!(
                    skill = %skill.name,
                    error = %e,
                    "skill execution IO error"
                );
                Err(ExecutorError::Io(e))
            }
            // Process completed
            Ok(Ok(status)) => {
                let stdout = read_output(stdout_handle).await;
                let stderr = read_output(stderr_handle).await;
                let exit_code = status.code();
                let success = status.success();

                if !success {
                    tracing::warn!(
                        skill = %skill.name,
                        exit_code = ?exit_code,
                        "skill exited with non-zero code"
                    );
                    return Err(ExecutorError::NonZeroExitCode {
                        skill_name: skill.name.clone(),
                        exit_code: exit_code.unwrap_or(-1),
                        stdout,
                        stderr,
                    });
                }

                tracing::info!(
                    skill = %skill.name,
                    duration_ms,
                    "skill execution completed successfully"
                );

                Ok(ExecutionResult {
                    stdout,
                    stderr,
                    exit_code,
                    duration_ms,
                    timed_out: false,
                    success: true,
                })
            }
        }
    }

    /// Execute an MCP-backed skill by routing its input to an external MCP
    /// server's tool over JSON-RPC 2.0 stdio.
    ///
    /// Called early from `execute()` when `skill.mcp` is `Some`. The external
    /// server is launched inside the A4 (P32) sandbox so untrusted MCP code
    /// runs with the same CWD + policy isolation as local skills. B-2 检查点
    /// 结论: 该路径可行, 用最小 JSON-RPC 客户端实现, 不引入重型 SDK。
    async fn execute_mcp_skill(
        &self,
        skill: &Skill,
        mcp: &McpRef,
        input: &Value,
        start: Instant,
    ) -> ExecutorResult<ExecutionResult> {
        tracing::info!(
            skill = %skill.name,
            server = ?mcp.server_command,
            tool = %mcp.tool,
            "executing MCP-backed skill"
        );

        let result = crate::skill::mcp::run_mcp_tool(&mcp.server_command, &mcp.tool, input)
            .await
            .map_err(|e| ExecutorError::ExecutionFailed(e.to_string()))?;

        let stdout = serde_json::to_string(&result).unwrap_or_default();
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(ExecutionResult {
            stdout,
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms,
            timed_out: false,
            success: true,
        })
    }
}

impl std::fmt::Debug for Executor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Executor")
            .field("config", &self.config)
            .field("max_concurrent", &self.pool.max_concurrent())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Wrap a command with ulimit for memory limiting.
///
/// Returns (executable, args) for `sh -c "ulimit -v {kb}; exec {cmd}"`.
///
/// This is only used for non-Shell runtimes. Shell runtimes skip this
/// wrapping to avoid nested shell layers.
fn wrap_with_ulimit(exec: &str, args: &[String], memory_limit_mb: u32) -> (String, Vec<String>) {
    let memory_kb = memory_limit_mb * 1024;
    let mut cmd_str = format!("ulimit -v {};", memory_kb);
    cmd_str.push_str(" exec ");
    cmd_str.push_str(&shell_quote(exec));
    for arg in args {
        cmd_str.push(' ');
        cmd_str.push_str(&shell_quote(arg));
    }
    ("sh".to_string(), vec!["-c".to_string(), cmd_str])
}

/// Quote a string for safe use in a shell command.
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Safe characters that don't need quoting
    if s.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | '~'))
    {
        return s.to_string();
    }
    // Single-quote and escape embedded single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Read output from a child process handle, if present.
async fn read_output<R: AsyncRead + Unpin>(handle: Option<R>) -> String {
    match handle {
        Some(mut h) => {
            let mut buf = Vec::new();
            let _ = h.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        }
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::{SkillPermissions, SkillRuntime, SkillRuntimeType};
    use std::path::PathBuf;

    // --- Test helpers ---

    /// Build a test skill. By default grants `shell: true` so the existing
    /// Shell-runtime happy-path tests continue to exercise real execution;
    /// denial-path tests construct their own `permissions` explicitly.
    fn make_skill(name: &str, runtime_type: SkillRuntimeType, entry: &str, path: &str) -> Skill {
        Skill {
            schema_version: "1.0".to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Test skill {name}"),
            category: "test".to_string(),
            trigger_phrases: vec!["test".to_string()],
            runtime: SkillRuntime {
                runtime_type,
                entry: entry.to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            permissions: SkillPermissions {
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

    fn is_command_available(cmd: &str) -> bool {
        std::process::Command::new(cmd)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    // --- Config tests ---

    #[test]
    fn test_config_defaults() {
        let config = ExecutionConfig::default();
        assert_eq!(config.max_concurrent, 4);
        assert_eq!(config.default_timeout, 60);
        assert_eq!(config.default_memory_limit_mb, 256);
        assert!(config.capture_stderr);
    }

    #[test]
    fn test_config_serialization() {
        let config = ExecutionConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ExecutionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_concurrent, config.max_concurrent);
        assert_eq!(deserialized.default_timeout, config.default_timeout);
    }

    #[test]
    fn test_executor_debug() {
        let executor = Executor::with_defaults();
        let debug = format!("{executor:?}");
        assert!(debug.contains("Executor"));
    }

    // --- Helper function tests ---

    #[test]
    fn test_shell_quote_simple() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("/usr/bin/python3"), "/usr/bin/python3");
    }

    #[test]
    fn test_shell_quote_with_spaces() {
        let quoted = shell_quote("/path with spaces/script.py");
        assert!(quoted.starts_with('\''));
        assert!(quoted.ends_with('\''));
        assert!(quoted.contains("/path with spaces/script.py"));
    }

    #[test]
    fn test_shell_quote_empty() {
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_wrap_with_ulimit() {
        let (exec, args) = wrap_with_ulimit("python3", &["script.py".to_string()], 256);
        assert_eq!(exec, "sh");
        assert_eq!(args[0], "-c");
        assert!(args[1].contains("ulimit -v"));
        assert!(args[1].contains("262144")); // 256 * 1024
        assert!(args[1].contains("python3"));
        assert!(args[1].contains("script.py"));
    }

    // --- Python execution tests ---

    #[tokio::test]
    async fn test_execute_python_success() {
        if !is_command_available("python3") {
            eprintln!("python3 not available — skipping test");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let script = r#"import sys, json
data = json.load(sys.stdin)
print(json.dumps(data))"#;
        std::fs::write(tmp.path().join("script.py"), script).unwrap();

        let skill = make_skill(
            "echo_python",
            SkillRuntimeType::Python,
            "script.py",
            tmp.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({"message": "hello"});

        let result = executor.execute(&skill, &input).await.unwrap();
        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.timed_out);
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_python_non_zero_exit() {
        if !is_command_available("python3") {
            eprintln!("python3 not available — skipping test");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let script = r#"import sys
print("error message", file=sys.stderr)
sys.exit(1)"#;
        std::fs::write(tmp.path().join("script.py"), script).unwrap();

        let skill = make_skill(
            "fail_python",
            SkillRuntimeType::Python,
            "script.py",
            tmp.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({});

        let result = executor.execute(&skill, &input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutorError::NonZeroExitCode {
                exit_code, stderr, ..
            } => {
                assert_eq!(exit_code, 1);
                assert!(stderr.contains("error message"));
            }
            other => panic!("expected NonZeroExitCode, got {other:?}"),
        }
    }

    // --- Shell execution tests ---

    #[tokio::test]
    async fn test_execute_shell_success() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\ncat\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        let skill = make_skill(
            "echo_shell",
            SkillRuntimeType::Shell,
            "run.sh",
            tmp.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({"message": "hello"});

        let result = executor.execute(&skill, &input).await.unwrap();
        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.timed_out);
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_shell_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\nsleep 30\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        let mut skill = make_skill(
            "slow_shell",
            SkillRuntimeType::Shell,
            "run.sh",
            tmp.path().to_str().unwrap(),
        );
        skill.runtime.timeout = 1; // 1 second timeout

        let executor = Executor::with_defaults();
        let input = serde_json::json!({});

        let result = executor.execute(&skill, &input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutorError::Timeout {
                skill_name,
                timeout_secs,
            } => {
                assert_eq!(skill_name, "slow_shell");
                assert_eq!(timeout_secs, 1);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_shell_non_zero_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\necho 'error' >&2\nexit 1\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        let skill = make_skill(
            "fail_shell",
            SkillRuntimeType::Shell,
            "run.sh",
            tmp.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({});

        let result = executor.execute(&skill, &input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutorError::NonZeroExitCode {
                exit_code, stderr, ..
            } => {
                assert_eq!(exit_code, 1);
                assert!(stderr.contains("error"));
            }
            other => panic!("expected NonZeroExitCode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_entry_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let skill = make_skill(
            "missing_entry",
            SkillRuntimeType::Shell,
            "nonexistent.sh",
            tmp.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({});

        let result = executor.execute(&skill, &input).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ExecutorError::EntryNotFound { .. }
        ));
    }

    #[tokio::test]
    async fn test_execute_runtime_not_found() {
        // Test with JavaScript — if node is not installed, we get RuntimeNotFound
        if is_command_available("node") {
            eprintln!("node is available — skipping RuntimeNotFound test");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("script.js"), "console.log('hello')").unwrap();

        let skill = make_skill(
            "test_js",
            SkillRuntimeType::Javascript,
            "script.js",
            tmp.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({});

        let result = executor.execute(&skill, &input).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ExecutorError::RuntimeNotFound { .. }
        ));
    }

    // --- RustBinary execution tests ---

    #[tokio::test]
    async fn test_execute_rust_binary_success() {
        let cat_dir = if std::path::Path::new("/bin/cat").exists() {
            "/bin"
        } else if std::path::Path::new("/usr/bin/cat").exists() {
            "/usr/bin"
        } else {
            eprintln!("cat not found — skipping test");
            return;
        };

        let skill = make_skill("cat_binary", SkillRuntimeType::RustBinary, "cat", cat_dir);
        let executor = Executor::with_defaults();
        let input = serde_json::json!({"message": "hello"});

        let result = executor.execute(&skill, &input).await.unwrap();
        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("hello"));
    }

    // --- P32 sandbox: permission denial tests ---

    #[tokio::test]
    async fn test_execute_shell_denied_when_shell_false() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\ncat\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        let mut skill = make_skill(
            "denied_shell",
            SkillRuntimeType::Shell,
            "run.sh",
            tmp.path().to_str().unwrap(),
        );
        // Override the helper default: this skill is NOT allowed to run shell.
        skill.permissions = SkillPermissions {
            shell: false,
            ..Default::default()
        };

        let executor = Executor::with_defaults();
        let input = serde_json::json!({});

        let result = executor.execute(&skill, &input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutorError::PermissionDenied { skill_name, .. } => {
                assert_eq!(skill_name, "denied_shell");
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_shell_allowed_when_shell_true() {
        let tmp = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\ncat\n";
        std::fs::write(tmp.path().join("run.sh"), script).unwrap();

        // make_skill grants shell:true, so this should execute normally.
        let skill = make_skill(
            "allowed_shell",
            SkillRuntimeType::Shell,
            "run.sh",
            tmp.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({"message": "hello"});

        let result = executor.execute(&skill, &input).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("hello"));
    }

    // --- P32 sandbox: write-isolation test ---

    #[tokio::test]
    async fn test_execute_writes_to_sandbox_not_skill_dir() {
        if !is_command_available("python3") {
            eprintln!("python3 not available — skipping test");
            return;
        }

        let skill_dir = tempfile::tempdir().unwrap();
        // Script writes a relative file (no absolute path) to its CWD.
        let script = r#"import os, json, sys
# Write a probe file using only a relative name -> must land in the sandbox CWD.
with open("probe.txt", "w") as f:
    f.write("written-in-sandbox")
print(json.dumps({
    "sandbox": os.environ.get("CASPIAN_SANDBOX", ""),
    "skill_dir": os.environ.get("CASPIAN_SKILL_DIR", ""),
}))"#;
        std::fs::write(skill_dir.path().join("script.py"), script).unwrap();

        let skill = make_skill(
            "isolated_python",
            SkillRuntimeType::Python,
            "script.py",
            skill_dir.path().to_str().unwrap(),
        );
        let executor = Executor::with_defaults();
        let input = serde_json::json!({});

        let result = executor.execute(&skill, &input).await.unwrap();
        assert!(result.success);

        // The probe file must NOT appear in the skill's own directory.
        assert!(
            !skill_dir.path().join("probe.txt").exists(),
            "relative write leaked into the skill directory instead of the sandbox"
        );

        // The reported sandbox path must differ from the skill directory.
        let reported: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_ne!(reported["sandbox"], reported["skill_dir"]);
        assert!(reported["sandbox"].as_str().unwrap().starts_with("/tmp")
            || reported["sandbox"].as_str().unwrap().contains("temp"));
    }

    // --- JavaScript execution test ---

    #[tokio::test]
    async fn test_execute_javascript_success() {
        if !is_command_available("node") {
            eprintln!("node not available — skipping test");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let script = r#"let d='';
process.stdin.on('data',c=>d+=c);
process.stdin.on('end',()=>console.log(d));"#;
        std::fs::write(tmp.path().join("script.js"), script).unwrap();

        let skill = make_skill(
            "echo_js",
            SkillRuntimeType::Javascript,
            "script.js",
            tmp.path().to_str().unwrap(),
        );
        // Node.js/V8 requires significant virtual memory for its heap.
        // The default 256MB ulimit -v is too low for V8, so disable
        // memory limiting for this test.
        let config = ExecutionConfig {
            default_memory_limit_mb: 0,
            ..ExecutionConfig::default()
        };
        let executor = Executor::new(config);
        let input = serde_json::json!({"message": "hello"});

        let result = executor.execute(&skill, &input).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("hello"));
    }

    // --- Result structure test ---

    #[test]
    fn test_execution_result_fields() {
        let result = ExecutionResult {
            stdout: "hello".to_string(),
            stderr: "world".to_string(),
            exit_code: Some(0),
            duration_ms: 100,
            timed_out: false,
            success: true,
        };
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "world");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.duration_ms, 100);
        assert!(!result.timed_out);
        assert!(result.success);
    }
}
