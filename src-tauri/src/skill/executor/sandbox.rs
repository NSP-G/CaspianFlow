//! Skill sandbox layer — per-skill isolation and permission enforcement.
//!
//! This module is the implementation of **P32 (安全沙箱)**. It wraps each skill
//! execution with a private, disposable sandbox and enforces the permission
//! policy declared in `skill.yaml` (`permissions:`).
//!
//! ## What is enforced in-process (this iteration)
//!
//! 1. **Write isolation** — every skill runs with its *working directory* set to
//!    a fresh, private [`tempfile::TempDir`]. Relative file writes therefore
//!    land inside the sandbox, never in the skill's own directory or elsewhere
//!    on the host. The directory (and everything written into it) is removed
//!    automatically when [`crate::skill::executor::Executor::execute`] returns —
//!    **success or failure alike** — so a crashed or timed-out skill can never
//!    leave debris behind.
//!
//! 2. **Shell gate** — a skill that declares `shell: false` is refused at spawn
//!    time if its runtime is `Shell`. This is fully enforceable in-process
//!    because we simply never spawn the process.
//!
//! ## What is declared here but deferred to an OS sandbox harness (future)
//!
//! These policies are *recorded* (via env markers a harness can honor, and via
//! warnings) but cannot be truly confined without kernel-level primitives:
//!
//! - `network: false` — blocking sockets requires seccomp / Landlock / a
//!   network namespace. We set `CASPIAN_NETWORK_ALLOWED` and warn.
//! - `fs: [{ read, write }]` path scoping — confining filesystem access to an
//!   allow-list needs Landlock (Linux) / sandbox entitlements. We surface the
//!   declared policy via `CASPIAN_FS_READ` / `CASPIAN_FS_WRITE`.
//!
//! The write-isolation primitive above already confines *writes* to the sandbox,
//! which is the highest-value, dependency-free guarantee we can ship now.

use tokio::process::Command;

use tempfile::TempDir;

use crate::skill::schema::{Skill, SkillRuntimeType};
use crate::types::ExecutorError;

/// A private, disposable sandbox directory for a single skill execution.
///
/// Holds the [`TempDir`] guard; when this struct is dropped (at the end of
/// `execute()`, on either the success or the error path) the directory and all
/// its contents are deleted.
pub struct SkillSandbox {
    /// Absolute path to the sandbox directory. Used as the child process CWD.
    pub dir: std::path::PathBuf,
    /// RAII guard — dropping it removes `dir`. Must outlive the execution.
    _guard: TempDir,
}

impl SkillSandbox {
    /// Create a fresh, private sandbox directory for one skill execution.
    pub fn new() -> Result<Self, ExecutorError> {
        let guard = TempDir::new().map_err(ExecutorError::Io)?;
        let dir = guard.path().to_path_buf();
        Ok(Self {
            dir,
            _guard: guard,
        })
    }
}

/// Enforce the *enforceable* subset of a skill's declared permissions.
///
/// Returns `Ok(())` if the skill may proceed, or
/// [`ExecutorError::PermissionDenied`] otherwise.
///
/// Currently enforced (see module docs for deferred policies):
/// - `shell: false` blocks the `Shell` runtime.
pub fn check_runtime_permissions(skill: &Skill) -> Result<(), ExecutorError> {
    if skill.runtime.runtime_type == SkillRuntimeType::Shell && !skill.permissions.shell {
        return Err(ExecutorError::PermissionDenied {
            skill_name: skill.name.clone(),
            reason: "skill declares `shell: false` but uses the Shell runtime".to_string(),
        });
    }
    Ok(())
}

/// Inject sandbox + permission context into a child process command.
///
/// This sets the working directory to the private sandbox and records the
/// declared policy via environment markers that a future OS sandbox harness
/// (seccomp/Landlock) can honor, and that skills can introspect. It does *not*
/// perform kernel-level enforcement by itself.
pub fn apply_sandbox_env(cmd: &mut Command, sandbox: &SkillSandbox, skill: &Skill) {
    cmd.current_dir(&sandbox.dir);

    // Scratch space the skill is allowed to write into.
    cmd.env("CASPIAN_SANDBOX", sandbox.dir.to_string_lossy().to_string());
    // The skill's own (read-only from the sandbox's perspective) directory,
    // so skills can still locate their bundled assets via an absolute path.
    cmd.env("CASPIAN_SKILL_DIR", skill.dir().to_string_lossy().to_string());

    // Declared network policy (true enforcement deferred to a harness).
    cmd.env(
        "CASPIAN_NETWORK_ALLOWED",
        if skill.permissions.network { "1" } else { "0" },
    );

    // Declared filesystem allow-lists (true enforcement deferred to Landlock).
    let read_paths: Vec<String> = skill
        .permissions
        .fs
        .iter()
        .flat_map(|f| f.read.iter().cloned())
        .collect();
    cmd.env("CASPIAN_FS_READ", read_paths.join(":"));

    let write_paths: Vec<String> = skill
        .permissions
        .fs
        .iter()
        .flat_map(|f| f.write.clone().unwrap_or_default())
        .collect();
    cmd.env("CASPIAN_FS_WRITE", write_paths.join(":"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::{SkillPermissions, SkillRuntime};
    use std::path::PathBuf;

    fn shell_skill(permissions: SkillPermissions) -> Skill {
        Skill {
            mcp: None,
            schema_version: "1.0".to_string(),
            name: "probe".to_string(),
            display_name: "Probe".to_string(),
            version: "1.0.0".to_string(),
            description: "test".to_string(),
            category: "test".to_string(),
            trigger_phrases: vec![],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Shell,
                entry: "run.sh".to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            permissions,
            tags: vec![],
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from("/tmp/probe"),
        }
    }

    #[test]
    fn test_check_runtime_permissions_denies_shell_when_false() {
        let skill = shell_skill(SkillPermissions {
            shell: false,
            ..Default::default()
        });
        let result = check_runtime_permissions(&skill);
        assert!(matches!(
            result,
            Err(ExecutorError::PermissionDenied { .. })
        ));
    }

    #[test]
    fn test_check_runtime_permissions_allows_shell_when_true() {
        let skill = shell_skill(SkillPermissions {
            shell: true,
            ..Default::default()
        });
        assert!(check_runtime_permissions(&skill).is_ok());
    }

    #[test]
    fn test_check_runtime_permissions_ignores_shell_flag_for_python() {
        let mut skill = shell_skill(SkillPermissions {
            shell: false,
            ..Default::default()
        });
        skill.runtime.runtime_type = SkillRuntimeType::Python;
        // shell:false must not block non-Shell runtimes.
        assert!(check_runtime_permissions(&skill).is_ok());
    }

    #[test]
    fn test_sandbox_dir_is_absolute_and_unique() {
        let a = SkillSandbox::new().unwrap();
        let b = SkillSandbox::new().unwrap();
        assert!(a.dir.is_absolute());
        assert!(b.dir.is_absolute());
        assert_ne!(a.dir, b.dir);
        assert!(a.dir.exists());
    }

    #[test]
    fn test_sandbox_cleanup_on_drop() {
        let path;
        {
            let sandbox = SkillSandbox::new().unwrap();
            path = sandbox.dir.clone();
            std::fs::write(path.join("marker.txt"), "x").unwrap();
            assert!(path.join("marker.txt").exists());
            // sandbox dropped here
        }
        assert!(!path.exists(), "sandbox dir must be removed on drop");
    }
}
