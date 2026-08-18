//! Rust binary runtime adapter.
//!
//! Executes pre-compiled binaries directly (no interpreter needed).

use crate::skill::schema::{Skill, SkillRuntimeType};
use crate::types::{ExecutorError, ExecutorResult};

use super::adapter::RuntimeAdapter;

/// Adapter for executing Rust binary skills.
///
/// The entry file is a pre-compiled executable that is run directly.
/// No interpreter is needed — `check_available` is a no-op.
/// Input JSON is written to stdin.
pub struct RustBinaryRuntimeAdapter;

#[async_trait::async_trait]
impl RuntimeAdapter for RustBinaryRuntimeAdapter {
    async fn check_available(&self) -> ExecutorResult<()> {
        // RustBinary doesn't need an external runtime — the binary itself
        // is the runtime. Availability is checked in build_args.
        Ok(())
    }

    fn build_args(&self, skill: &Skill) -> ExecutorResult<(String, Vec<String>)> {
        let entry_path = skill.entry_path();
        if !entry_path.exists() {
            return Err(ExecutorError::EntryNotFound {
                path: entry_path.display().to_string(),
            });
        }
        // Execute the binary directly with no arguments
        Ok((entry_path.to_string_lossy().to_string(), Vec::new()))
    }

    fn runtime_type(&self) -> SkillRuntimeType {
        SkillRuntimeType::RustBinary
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
            name: "test_binary".to_string(),
            display_name: "Test Binary".to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            category: "test".to_string(),
            trigger_phrases: vec![],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::RustBinary,
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
        let adapter = RustBinaryRuntimeAdapter;
        let skill = make_skill("nonexistent_bin", "/tmp");
        let result = adapter.build_args(&skill);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ExecutorError::EntryNotFound { .. }
        ));
    }

    #[test]
    fn test_build_args_success() {
        // Use /bin/cat or /usr/bin/cat as a test binary
        let (entry, path) = if std::path::Path::new("/bin/cat").exists() {
            ("cat", "/bin")
        } else if std::path::Path::new("/usr/bin/cat").exists() {
            ("cat", "/usr/bin")
        } else {
            return; // skip if cat not found
        };

        let skill = make_skill(entry, path);
        let adapter = RustBinaryRuntimeAdapter;
        let (exec, args) = adapter.build_args(&skill).unwrap();
        assert!(exec.ends_with("cat"));
        assert!(args.is_empty());
    }

    #[test]
    fn test_runtime_type() {
        let adapter = RustBinaryRuntimeAdapter;
        assert_eq!(adapter.runtime_type(), SkillRuntimeType::RustBinary);
    }
}
