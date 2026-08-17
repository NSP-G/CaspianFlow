//! Built-in atomic skills — shipped with CaspianFlow and installed on first run.
//!
//! ## Overview
//!
//! Five core skills are embedded directly in the binary as `&'static str`
//! constants:
//!
//! | Skill | Category | Description |
//! |-------|----------|-------------|
//! | `read_file` | file-system | Read a local text file |
//! | `write_file` | file-system | Write content to a local file |
//! | `shell_command` | system | Execute a shell command |
//! | `http_request` | network | Send an HTTP request |
//! | `summarize_text` | text | Extractive text summarization (stub for P23) |
//!
//! ## Installation
//!
//! [`install_builtin_skills`] is called by `SkillManager::init()` before
//! scanning. It is idempotent: if a skill directory already contains
//! `skill.yaml`, the skill is skipped (with a log) to allow user
//! customizations.
//!
//! ## Design notes
//!
//! - `{workspace}` in permission declarations is a literal string — the Skill
//!   itself does not parse it. Actual enforcement is handled by the P32
//!   sandbox layer.
//! - `summarize_text` uses an extractive stub (sentence selection). When P23
//!   (model adapter) is ready, the `extractive_summary()` function will be
//!   replaced with an LLM-based call. The `language` parameter is a
//!   placeholder for P23 model/prompt selection.
//! - `shell: true` in `shell_command` permissions is a declaration only.
//!   P32 sandbox will enforce it when ready.

use std::path::Path;

use crate::types::AppResult;

pub mod http_request;
pub mod read_file;
pub mod shell_command;
pub mod summarize_text;
pub mod write_file;

// --- System skill package (P37) ---
pub mod code_interpreter;
pub mod file_reader;
pub mod file_search;
pub mod file_writer;
pub mod json_parser;
pub mod memory_manager;
pub mod note_taker;
pub mod shell_runner;
pub mod skill_manager;
pub mod system_info;
pub mod web_fetcher;
pub mod workflow_runner;

/// Names of all built-in skills, in installation order.
pub const BUILTIN_SKILL_NAMES: &[&str] = &[
    // Core built-ins (pre-P37)
    "read_file",
    "write_file",
    "shell_command",
    "http_request",
    "summarize_text",
    // System skill package (P37)
    "file-reader",
    "file-writer",
    "file-search",
    "web-fetcher",
    "shell-runner",
    "system-info",
    "code-interpreter",
    "json-parser",
    "note-taker",
    "memory-manager",
    "skill-manager",
    "workflow-runner",
];

/// Install all built-in skills into the given skills directory.
///
/// Called by `SkillManager::init()` before scanning. Idempotent: if a skill
/// directory already contains `skill.yaml`, it is skipped (with a log) to
/// allow user customizations.
pub fn install_builtin_skills(skills_dir: &Path) -> AppResult<()> {
    std::fs::create_dir_all(skills_dir)?;

    install_skill(
        skills_dir,
        "read_file",
        read_file::SKILL_YAML,
        read_file::SCRIPT_PY,
        read_file::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "write_file",
        write_file::SKILL_YAML,
        write_file::SCRIPT_PY,
        write_file::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "shell_command",
        shell_command::SKILL_YAML,
        shell_command::SCRIPT_PY,
        shell_command::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "http_request",
        http_request::SKILL_YAML,
        http_request::SCRIPT_PY,
        http_request::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "summarize_text",
        summarize_text::SKILL_YAML,
        summarize_text::SCRIPT_PY,
        summarize_text::EXAMPLE_BASIC,
    )?;

    // --- System skill package (P37) ---
    install_skill(
        skills_dir,
        "file-reader",
        file_reader::SKILL_YAML,
        file_reader::SCRIPT_PY,
        file_reader::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "file-writer",
        file_writer::SKILL_YAML,
        file_writer::SCRIPT_PY,
        file_writer::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "file-search",
        file_search::SKILL_YAML,
        file_search::SCRIPT_PY,
        file_search::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "web-fetcher",
        web_fetcher::SKILL_YAML,
        web_fetcher::SCRIPT_PY,
        web_fetcher::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "shell-runner",
        shell_runner::SKILL_YAML,
        shell_runner::SCRIPT_PY,
        shell_runner::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "system-info",
        system_info::SKILL_YAML,
        system_info::SCRIPT_PY,
        system_info::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "code-interpreter",
        code_interpreter::SKILL_YAML,
        code_interpreter::SCRIPT_PY,
        code_interpreter::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "json-parser",
        json_parser::SKILL_YAML,
        json_parser::SCRIPT_PY,
        json_parser::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "note-taker",
        note_taker::SKILL_YAML,
        note_taker::SCRIPT_PY,
        note_taker::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "memory-manager",
        memory_manager::SKILL_YAML,
        memory_manager::SCRIPT_PY,
        memory_manager::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "skill-manager",
        skill_manager::SKILL_YAML,
        skill_manager::SCRIPT_PY,
        skill_manager::EXAMPLE_BASIC,
    )?;
    install_skill(
        skills_dir,
        "workflow-runner",
        workflow_runner::SKILL_YAML,
        workflow_runner::SCRIPT_PY,
        workflow_runner::EXAMPLE_BASIC,
    )?;

    Ok(())
}

/// Install a single skill into the skills directory.
///
/// Creates the directory structure and writes `skill.yaml`, `script.py`,
/// and `examples/01_basic.md`. If `skill.yaml` already exists, the skill
/// is skipped (idempotent) and a log message is emitted.
fn install_skill(
    skills_dir: &Path,
    name: &str,
    yaml: &str,
    script: &str,
    example: &str,
) -> AppResult<()> {
    let skill_dir = skills_dir.join(name);
    let yaml_path = skill_dir.join("skill.yaml");

    // Idempotent: skip if already exists
    if yaml_path.exists() {
        tracing::info!(
            skill = name,
            "built-in skill already exists, skipping (user may have customized it)"
        );
        return Ok(());
    }

    // Create directory structure
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::create_dir_all(skill_dir.join("examples"))?;
    std::fs::create_dir_all(skill_dir.join("assets"))?;

    // Write files
    std::fs::write(&yaml_path, yaml)?;
    std::fs::write(skill_dir.join("script.py"), script)?;
    std::fs::write(skill_dir.join("examples").join("01_basic.md"), example)?;

    tracing::info!(skill = name, "built-in skill installed");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::executor::Executor;
    use crate::skill::schema::Skill;
    use crate::skill::validator;
    use crate::types::ExecutorError;

    // --- Helpers ---

    fn is_python3_available() -> bool {
        std::process::Command::new("python3")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Install all built-in skills to a temp directory and return the path.
    fn setup_skills(tmp: &tempfile::TempDir) -> std::path::PathBuf {
        let skills_dir = tmp.path().join("skills");
        install_builtin_skills(&skills_dir).unwrap();
        skills_dir
    }

    // --- Installation tests ---

    #[test]
    fn test_install_creates_all_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        install_builtin_skills(&skills_dir).unwrap();

        for name in BUILTIN_SKILL_NAMES {
            let skill_dir = skills_dir.join(name);
            assert!(
                skill_dir.join("skill.yaml").exists(),
                "skill.yaml missing for {name}"
            );
            assert!(
                skill_dir.join("script.py").exists(),
                "script.py missing for {name}"
            );
            assert!(
                skill_dir.join("examples").join("01_basic.md").exists(),
                "examples/01_basic.md missing for {name}"
            );
            assert!(
                skill_dir.join("assets").exists(),
                "assets/ dir missing for {name}"
            );
        }
    }

    #[test]
    fn test_install_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        // First install
        install_builtin_skills(&skills_dir).unwrap();

        // Modify a file to simulate user customization
        let yaml_path = skills_dir.join("read_file").join("skill.yaml");
        let original = std::fs::read_to_string(&yaml_path).unwrap();
        let modified = original.replace("Read File", "My Custom Reader");
        std::fs::write(&yaml_path, &modified).unwrap();

        // Second install — should not overwrite
        install_builtin_skills(&skills_dir).unwrap();

        // Verify the modification is preserved
        let after = std::fs::read_to_string(&yaml_path).unwrap();
        assert!(
            after.contains("My Custom Reader"),
            "user customization was overwritten"
        );
    }

    #[test]
    fn test_install_creates_skills_dir_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("nested").join("skills");

        // Directory doesn't exist yet
        assert!(!skills_dir.exists());

        install_builtin_skills(&skills_dir).unwrap();

        assert!(skills_dir.exists());
        assert!(skills_dir.join("read_file").join("skill.yaml").exists());
    }

    // --- Validation tests ---

    #[test]
    fn test_all_skill_yaml_valid() {
        let skills: &[(&str, &str)] = &[
            ("read_file", read_file::SKILL_YAML),
            ("write_file", write_file::SKILL_YAML),
            ("shell_command", shell_command::SKILL_YAML),
            ("http_request", http_request::SKILL_YAML),
            ("summarize_text", summarize_text::SKILL_YAML),
            // System skill package (P37)
            ("file-reader", file_reader::SKILL_YAML),
            ("file-writer", file_writer::SKILL_YAML),
            ("file-search", file_search::SKILL_YAML),
            ("web-fetcher", web_fetcher::SKILL_YAML),
            ("shell-runner", shell_runner::SKILL_YAML),
            ("system-info", system_info::SKILL_YAML),
            ("code-interpreter", code_interpreter::SKILL_YAML),
            ("json-parser", json_parser::SKILL_YAML),
            ("note-taker", note_taker::SKILL_YAML),
            ("memory-manager", memory_manager::SKILL_YAML),
            ("skill-manager", skill_manager::SKILL_YAML),
            ("workflow-runner", workflow_runner::SKILL_YAML),
        ];

        for (name, yaml) in skills {
            let skill =
                Skill::from_yaml(yaml).unwrap_or_else(|e| panic!("failed to parse {name}: {e}"));
            assert_eq!(skill.name, *name);
            assert!(
                !skill.description.is_empty(),
                "description empty for {name}"
            );
            assert!(!skill.category.is_empty(), "category empty for {name}");
            assert!(
                !skill.trigger_phrases.is_empty(),
                "trigger_phrases empty for {name}"
            );

            validator::validate(&skill)
                .unwrap_or_else(|e| panic!("validation failed for {name}: {e}"));
        }
    }

    #[test]
    fn test_permissions_declared_correctly() {
        let read_file_skill = Skill::from_yaml(read_file::SKILL_YAML).unwrap();
        assert!(!read_file_skill.permissions.fs.is_empty());
        assert!(!read_file_skill.permissions.network);
        assert!(!read_file_skill.permissions.shell);

        let http_skill = Skill::from_yaml(http_request::SKILL_YAML).unwrap();
        assert!(http_skill.permissions.network);
        assert!(!http_skill.permissions.shell);

        let shell_skill = Skill::from_yaml(shell_command::SKILL_YAML).unwrap();
        assert!(!shell_skill.permissions.network);
        assert!(shell_skill.permissions.shell);

        let summarize_skill = Skill::from_yaml(summarize_text::SKILL_YAML).unwrap();
        assert!(!summarize_skill.permissions.network);
        assert!(!summarize_skill.permissions.shell);
    }

    // --- Execution tests (require python3) ---

    #[tokio::test]
    async fn test_execute_read_file() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        // Create a test file
        let test_file = tmp.path().join("input.txt");
        std::fs::write(&test_file, "Hello, CaspianFlow!").unwrap();

        // Load and execute
        let skill = Skill::load(&skills_dir.join("read_file").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();
        let input = serde_json::json!({
            "path": test_file.to_string_lossy().to_string()
        });
        let result = executor.execute(&skill, &input).await.unwrap();

        assert!(result.success);
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["content"].as_str().unwrap(), "Hello, CaspianFlow!");
        assert_eq!(output["size"].as_u64().unwrap(), 19);
        assert_eq!(output["encoding"].as_str().unwrap(), "utf-8");
    }

    #[tokio::test]
    async fn test_execute_read_file_not_found() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let skill = Skill::load(&skills_dir.join("read_file").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();
        let input = serde_json::json!({"path": "/nonexistent/file.txt"});
        let result = executor.execute(&skill, &input).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutorError::NonZeroExitCode {
                stdout, exit_code, ..
            } => {
                assert_eq!(exit_code, 1);
                let output: serde_json::Value = serde_json::from_str(&stdout).unwrap();
                assert!(output["error"].as_str().unwrap().contains("file not found"));
            }
            other => panic!("expected NonZeroExitCode, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_write_file() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let output_file = tmp.path().join("output.txt");

        let skill = Skill::load(&skills_dir.join("write_file").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();
        let input = serde_json::json!({
            "path": output_file.to_string_lossy().to_string(),
            "content": "Written by CaspianFlow!"
        });
        let result = executor.execute(&skill, &input).await.unwrap();

        assert!(result.success);
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["bytes_written"].as_u64().unwrap(), 23);
        assert!(!output["appended"].as_bool().unwrap());

        // Verify the file was actually written
        let content = std::fs::read_to_string(&output_file).unwrap();
        assert_eq!(content, "Written by CaspianFlow!");
    }

    #[tokio::test]
    async fn test_execute_write_file_append() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let output_file = tmp.path().join("append.txt");
        std::fs::write(&output_file, "Line 1\n").unwrap();

        let skill = Skill::load(&skills_dir.join("write_file").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();
        let input = serde_json::json!({
            "path": output_file.to_string_lossy().to_string(),
            "content": "Line 2\n",
            "append": true
        });
        let result = executor.execute(&skill, &input).await.unwrap();

        assert!(result.success);
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert!(output["appended"].as_bool().unwrap());

        // Verify appended content
        let content = std::fs::read_to_string(&output_file).unwrap();
        assert_eq!(content, "Line 1\nLine 2\n");
    }

    #[tokio::test]
    async fn test_execute_shell_command() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let skill = Skill::load(&skills_dir.join("shell_command").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();
        let input = serde_json::json!({
            "command": "echo",
            "args": ["Hello from shell_command"]
        });
        let result = executor.execute(&skill, &input).await.unwrap();

        assert!(result.success);
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["exit_code"].as_u64().unwrap(), 0);
        assert!(output["stdout"]
            .as_str()
            .unwrap()
            .contains("Hello from shell_command"));
    }

    #[tokio::test]
    async fn test_execute_shell_command_not_found() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let skill = Skill::load(&skills_dir.join("shell_command").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();
        let input = serde_json::json!({
            "command": "this_command_does_not_exist_12345"
        });
        let result = executor.execute(&skill, &input).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutorError::NonZeroExitCode { stdout, .. } => {
                let output: serde_json::Value = serde_json::from_str(&stdout).unwrap();
                assert!(output["error"]
                    .as_str()
                    .unwrap()
                    .contains("command not found"));
            }
            other => panic!("expected NonZeroExitCode, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_http_request_error() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let skill = Skill::load(&skills_dir.join("http_request").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();

        // Empty URL triggers immediate error — no network needed
        let input = serde_json::json!({"url": ""});
        let result = executor.execute(&skill, &input).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutorError::NonZeroExitCode {
                stdout, exit_code, ..
            } => {
                assert_eq!(exit_code, 1);
                let output: serde_json::Value = serde_json::from_str(&stdout).unwrap();
                assert_eq!(output["error"].as_str().unwrap(), "url is required");
            }
            other => panic!("expected NonZeroExitCode, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_summarize_text() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let skill = Skill::load(&skills_dir.join("summarize_text").join("skill.yaml")).unwrap();
        let executor = Executor::with_defaults();
        let input = serde_json::json!({
            "text": "First sentence. Second sentence. Third sentence. Fourth sentence.",
            "max_length": 30
        });
        let result = executor.execute(&skill, &input).await.unwrap();

        assert!(result.success);
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["method"].as_str().unwrap(), "extractive_stub");
        assert!(!output["summary"].as_str().unwrap().is_empty());
        assert!(output["original_length"].as_u64().unwrap() > 0);
        assert!(output["summary_length"].as_u64().unwrap() > 0);
        assert!(
            output["summary_length"].as_u64().unwrap()
                <= output["original_length"].as_u64().unwrap()
        );
    }

    // --- SkillManager integration test ---

    #[tokio::test]
    async fn test_skill_manager_loads_builtin_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        let manager = crate::skill::SkillManager::init(&skills_dir).await.unwrap();

        assert_eq!(manager.registry().count(), 17);
        for name in BUILTIN_SKILL_NAMES {
            assert!(
                manager.registry().exists(name),
                "skill {name} not in registry"
            );
        }
    }

    #[tokio::test]
    async fn test_skill_manager_builtin_skills_are_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        let manager = crate::skill::SkillManager::init(&skills_dir).await.unwrap();

        assert_eq!(manager.registry().count_enabled(), 17);
    }

    // --- P37 system skill package acceptance tests ---

    /// Load a freshly installed system skill by name into an Executor-ready Skill.
    fn load_system_skill(skills_dir: &std::path::Path, name: &str) -> Skill {
        Skill::load(&skills_dir.join(name).join("skill.yaml")).unwrap()
    }

    #[test]
    fn test_system_skill_package_count() {
        // 5 core built-ins + 12 system skills = 17 ≥ 10.
        assert_eq!(BUILTIN_SKILL_NAMES.len(), 17);
        let system_skills = &BUILTIN_SKILL_NAMES[5..];
        assert_eq!(system_skills.len(), 12);
    }

    #[tokio::test]
    async fn test_execute_file_reader() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);
        let file = tmp.path().join("sample.txt");
        std::fs::write(&file, "line one\nline two\nline three").unwrap();

        let skill = load_system_skill(&skills_dir, "file-reader");
        let executor = Executor::with_defaults();
        let input = serde_json::json!({"path": file.to_string_lossy().to_string()});
        let result = executor.execute(&skill, &input).await.unwrap();

        assert!(result.success);
        let out: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(out["content"].as_str().unwrap(), "line one\nline two\nline three");
        assert_eq!(out["line_count"].as_u64().unwrap(), 3);
        assert!(out["size"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_execute_shell_runner_echo() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let skill = load_system_skill(&skills_dir, "shell-runner");
        let executor = Executor::with_defaults();
        let input = serde_json::json!({"command": "echo", "args": ["hello"]});
        let result = executor.execute(&skill, &input).await.unwrap();

        assert!(result.success);
        let out: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(out["exit_code"].as_u64().unwrap(), 0);
        assert!(out["stdout"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_system_info() {
        if !is_python3_available() {
            eprintln!("skipping: python3 not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = setup_skills(&tmp);

        let skill = load_system_skill(&skills_dir, "system-info");
        let executor = Executor::with_defaults();
        let result = executor
            .execute(&skill, &serde_json::json!({}))
            .await
            .unwrap();

        assert!(result.success);
        let out: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert!(out["os"].is_string());
        assert!(out["python_version"].is_string());
        assert!(out["cpu_count"].is_number());
    }

    #[test]
    fn test_system_skills_permissions_consistent() {
        // Acceptance #6: permissions match each skill's described capability.
        let cases: &[(&str, &str)] = &[
            ("file-reader", file_reader::SKILL_YAML),
            ("file-writer", file_writer::SKILL_YAML),
            ("file-search", file_search::SKILL_YAML),
            ("web-fetcher", web_fetcher::SKILL_YAML),
            ("shell-runner", shell_runner::SKILL_YAML),
            ("system-info", system_info::SKILL_YAML),
            ("code-interpreter", code_interpreter::SKILL_YAML),
            ("json-parser", json_parser::SKILL_YAML),
            ("note-taker", note_taker::SKILL_YAML),
            ("memory-manager", memory_manager::SKILL_YAML),
            ("skill-manager", skill_manager::SKILL_YAML),
            ("workflow-runner", workflow_runner::SKILL_YAML),
        ];
        for (name, yaml) in cases {
            let skill = Skill::from_yaml(yaml)
                .unwrap_or_else(|e| panic!("parse failed for {name}: {e}"));
            match *name {
                "web-fetcher" => assert!(skill.permissions.network, "{name} must allow network"),
                "shell-runner" => assert!(skill.permissions.shell, "{name} must allow shell"),
                "file-reader" | "file-search" => {
                    assert!(!skill.permissions.fs.is_empty(), "{name} must allow fs read")
                }
                "file-writer" | "note-taker" | "memory-manager" => {
                    assert!(!skill.permissions.fs.is_empty(), "{name} must allow fs")
                }
                "system-info" | "code-interpreter" | "json-parser" | "skill-manager"
                | "workflow-runner" => {
                    assert!(!skill.permissions.network && !skill.permissions.shell,
                        "{name} must be offline & non-shell")
                }
                _ => {}
            }
            // Never declare both network AND shell (high-risk combo).
            assert!(
                !(skill.permissions.network && skill.permissions.shell),
                "{name} declares both network and shell"
            );
        }
    }
}
