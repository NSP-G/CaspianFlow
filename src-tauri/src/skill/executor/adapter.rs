//! Runtime adapter trait and factory.
//!
//! Each runtime type (Python, JavaScript, RustBinary, Shell) implements
//! [`RuntimeAdapter`], which knows how to check availability and build
//! the command arguments for that runtime.

use crate::skill::schema::{Skill, SkillRuntimeType};
use crate::types::ExecutorResult;

/// A runtime adapter knows how to build and execute commands for a specific
/// runtime type (Python, JavaScript, RustBinary, Shell).
///
/// The adapter is responsible for:
/// 1. Checking if the runtime is available on this system
/// 2. Building the (executable, args) tuple for the Executor to spawn
///
/// The Executor itself handles process spawning, stdin/stdout piping,
/// timeout, memory limits, and concurrency control.
#[async_trait::async_trait]
pub trait RuntimeAdapter: Send + Sync {
    /// Check if the runtime is available on this system.
    ///
    /// For interpreted runtimes (Python, JavaScript), this checks if the
    /// interpreter is installed. For RustBinary, this is a no-op (the
    /// binary itself IS the runtime).
    async fn check_available(&self) -> ExecutorResult<()>;

    /// Build the command arguments for this runtime.
    ///
    /// Returns `(executable, args)` that the Executor will use to spawn
    /// the process. The executable should be found in PATH or be an
    /// absolute path.
    fn build_args(&self, skill: &Skill) -> ExecutorResult<(String, Vec<String>)>;

    /// Get the runtime type this adapter handles.
    fn runtime_type(&self) -> SkillRuntimeType;
}

/// Create a runtime adapter for the given runtime type.
///
/// This is a factory function used by the [`Executor`](super::Executor) to
/// select the correct adapter based on the skill's runtime type.
pub fn create_adapter(runtime_type: SkillRuntimeType) -> Box<dyn RuntimeAdapter> {
    match runtime_type {
        SkillRuntimeType::Python => Box::new(super::python::PythonRuntimeAdapter),
        SkillRuntimeType::Javascript => Box::new(super::javascript::JavascriptRuntimeAdapter),
        SkillRuntimeType::RustBinary => Box::new(super::rust_binary::RustBinaryRuntimeAdapter),
        SkillRuntimeType::Shell => Box::new(super::shell::ShellRuntimeAdapter),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_adapter_python() {
        let adapter = create_adapter(SkillRuntimeType::Python);
        assert_eq!(adapter.runtime_type(), SkillRuntimeType::Python);
    }

    #[test]
    fn test_create_adapter_javascript() {
        let adapter = create_adapter(SkillRuntimeType::Javascript);
        assert_eq!(adapter.runtime_type(), SkillRuntimeType::Javascript);
    }

    #[test]
    fn test_create_adapter_rust_binary() {
        let adapter = create_adapter(SkillRuntimeType::RustBinary);
        assert_eq!(adapter.runtime_type(), SkillRuntimeType::RustBinary);
    }

    #[test]
    fn test_create_adapter_shell() {
        let adapter = create_adapter(SkillRuntimeType::Shell);
        assert_eq!(adapter.runtime_type(), SkillRuntimeType::Shell);
    }
}
