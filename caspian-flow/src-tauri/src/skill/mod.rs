//! Skill management module — the single entry point for all skill operations.
//!
//! ## Usage
//!
//! ```no_run
//! use caspian_flow::skill::SkillManager;
//! use std::path::Path;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let manager = SkillManager::init(Path::new("~/.caspian/skills")).await?;
//! let skills = manager.registry().list_all();
//! println!("loaded {} skills", skills.len());
//! # Ok(())
//! # }
//! ```

pub mod builtin;
pub mod examples;
pub mod executor;
pub mod mcp;
pub mod registry;
pub mod scanner;
pub mod schema;
pub mod skill_template;
pub mod source;
pub mod validator;

pub use examples::{load_examples, load_examples_indexed, SkillExample};
pub use registry::{SharedSkillRegistry, SkillRegistry};
pub use scanner::{ScanReport, SkillScanner};
pub use schema::{
    FsPermission, Skill, SkillPermissions, SkillRuntime, SkillRuntimeType, SKILL_SCHEMA_VERSION,
};
pub use skill_template::{create_skill_template, template_yaml, DEFAULT_SKILL_YAML};
pub use validator::{validate, validate_with_warnings, SkillValidationResult};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use crate::types::AppResult;

/// The central skill manager — owns the scanner and registry.
///
/// Typical lifecycle:
/// 1. `init()` — scans the skills directory and populates the registry.
/// 2. `reload()` — re-scans and replaces all skills in the registry.
/// 3. `registry()` — access the registry for queries and mutations.
pub struct SkillManager {
    registry: SharedSkillRegistry,
    scanner: SkillScanner,
    /// Most recent scan report (loaded skills + any missing/broken issues).
    /// Observable by the UI so module gaps are explicit, not silent (P30 WS1 §3).
    last_report: Mutex<Arc<ScanReport>>,
}

impl SkillManager {
    /// Initialize by scanning the given skills directory.
    ///
    /// Built-in skills are installed first (idempotent), then the directory
    /// is scanned to populate the registry.
    pub async fn init(skills_dir: &Path) -> AppResult<Self> {
        // Install built-in skills before scanning (idempotent — skips
        // existing skill.yaml files to allow user customizations)
        builtin::install_builtin_skills(skills_dir)?;

        let scanner = SkillScanner::new(skills_dir);
        let registry = Arc::new(SkillRegistry::new());

        let manager = Self {
            registry,
            scanner,
            last_report: Mutex::new(Arc::new(ScanReport::empty())),
        };
        manager.reload().await?;
        Ok(manager)
    }

    /// Re-scan the skills directory and replace all skills in the registry.
    ///
    /// Also records a [`ScanReport`] (including any missing/broken modules) so
    /// the UI can surface exactly what failed to load (P30 WS1 §3).
    pub async fn reload(&self) -> AppResult<()> {
        let report = self.scanner.scan().await;
        Arc::clone(&self.registry).replace_all(report.skills.clone());
        *self.last_report.lock() = Arc::new(ScanReport {
            skills: report.skills.clone(),
            issues: report.issues.clone(),
            scanned_dirs: report.scanned_dirs,
        });
        tracing::info!(
            total = self.registry.count(),
            enabled = self.registry.count_enabled(),
            issues = report.issues.len(),
            "skill registry reloaded"
        );
        Ok(())
    }

    /// Snapshot of the most recent scan: loaded skills plus any issues
    /// (missing/broken modules). Drives the UI resilience banner (P30 WS1).
    pub fn module_status(&self) -> Arc<ScanReport> {
        Arc::clone(&*self.last_report.lock())
    }

    /// Get a reference to the skill registry.
    pub fn registry(&self) -> &SharedSkillRegistry {
        &self.registry
    }

    /// Clone the shared (atomically-reference-counted) registry handle.
    ///
    /// Used by callers that need an owned `Arc<SkillRegistry>` — e.g. building
    /// a [`crate::workflow::WorkflowEngine`] (P28 run path).
    pub fn shared_registry(&self) -> SharedSkillRegistry {
        Arc::clone(&self.registry)
    }

    /// Get a reference to the skill scanner.
    pub fn scanner(&self) -> &SkillScanner {
        &self.scanner
    }

    /// Get the skills directory path.
    pub fn skills_dir(&self) -> &Path {
        self.scanner.skills_dir()
    }

    /// Load few-shot examples for a specific skill.
    pub fn load_examples(&self, skill_name: &str) -> Vec<SkillExample> {
        if let Some(skill) = self.registry.get(skill_name) {
            load_examples(&skill.path)
        } else {
            Vec::new()
        }
    }

    /// Create a new skill from the template.
    pub fn create_skill(&self, name: &str) -> AppResult<PathBuf> {
        let dir = self.scanner.skills_dir().join(name);
        create_skill_template(&dir, name)?;
        Ok(dir)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use super::scanner::ScanIssueKind;

    fn make_skill_yaml(name: &str, category: &str) -> String {
        format!(
            r#"schema_version: "1.0"
name: "{name}"
display_name: "{name}"
version: "1.0.0"
description: "Test skill {name}"
category: "{category}"

trigger_phrases:
  - "test {name}"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"

output_schema:
  type: "object"

permissions:
  fs: []
  network: false
  shell: false

tags:
  - "test"

author: "Test"
license: "MIT"
"#,
        )
    }

    #[tokio::test]
    async fn test_init_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        // All built-in skills are auto-installed
        assert_eq!(manager.registry().count(), builtin::BUILTIN_SKILL_NAMES.len());
    }

    #[tokio::test]
    async fn test_init_with_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        for name in ["skill_a", "skill_b"] {
            let dir = skills_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("skill.yaml"), make_skill_yaml(name, "test")).unwrap();
        }

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        // Built-in + 2 user skills
        assert_eq!(
            manager.registry().count(),
            builtin::BUILTIN_SKILL_NAMES.len() + 2
        );
        assert!(manager.registry().exists("skill_a"));
        assert!(manager.registry().exists("skill_b"));
    }

    #[tokio::test]
    async fn test_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let dir = skills_dir.join("skill_a");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("skill.yaml"), make_skill_yaml("skill_a", "test")).unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        // Built-in + 1 user skill
        assert_eq!(
            manager.registry().count(),
            builtin::BUILTIN_SKILL_NAMES.len() + 1
        );

        // Add a new skill
        let dir2 = skills_dir.join("skill_b");
        std::fs::create_dir_all(&dir2).unwrap();
        std::fs::write(dir2.join("skill.yaml"), make_skill_yaml("skill_b", "test")).unwrap();

        // Reload
        manager.reload().await.unwrap();
        // Built-in + 2 user skills
        assert_eq!(
            manager.registry().count(),
            builtin::BUILTIN_SKILL_NAMES.len() + 2
        );
    }

    #[tokio::test]
    async fn test_load_examples() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let skill_dir = skills_dir.join("test_skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.yaml"),
            make_skill_yaml("test_skill", "test"),
        )
        .unwrap();

        // Add examples
        let examples_dir = skill_dir.join("examples");
        std::fs::create_dir_all(&examples_dir).unwrap();
        std::fs::write(examples_dir.join("01_basic.md"), "example 1").unwrap();
        std::fs::write(examples_dir.join("02_adv.md"), "example 2").unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        let examples = manager.load_examples("test_skill");
        assert_eq!(examples.len(), 2);
        assert_eq!(examples[0].name, "01_basic");
    }

    #[tokio::test]
    async fn test_load_examples_nonexistent_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        let examples = manager.load_examples("nonexistent");
        assert!(examples.is_empty());
    }

    #[tokio::test]
    async fn test_create_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        let dir = manager.create_skill("new_skill").unwrap();

        assert!(dir.join("skill.yaml").exists());
        assert!(dir.join("script.py").exists());
        assert!(dir.join("examples").exists());
        assert!(dir.join("assets").exists());
    }

    #[tokio::test]
    async fn test_enable_disable_after_init() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let dir = skills_dir.join("test_skill");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("skill.yaml"),
            make_skill_yaml("test_skill", "test"),
        )
        .unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        // Built-in + 1 user skill, all enabled
        assert_eq!(
            manager.registry().count_enabled(),
            builtin::BUILTIN_SKILL_NAMES.len() + 1
        );

        manager.registry().disable("test_skill");
        // Built-in enabled, test_skill disabled
        assert_eq!(
            manager.registry().count_enabled(),
            builtin::BUILTIN_SKILL_NAMES.len()
        );

        manager.registry().enable("test_skill");
        // All enabled
        assert_eq!(
            manager.registry().count_enabled(),
            builtin::BUILTIN_SKILL_NAMES.len() + 1
        );
    }

    #[tokio::test]
    async fn test_full_workflow() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Create 3 skills with different categories
        let dir = skills_dir.join("read_file");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("skill.yaml"),
            make_skill_yaml("read_file", "file-system"),
        )
        .unwrap();

        let dir = skills_dir.join("write_file");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("skill.yaml"),
            make_skill_yaml("write_file", "file-system"),
        )
        .unwrap();

        let dir = skills_dir.join("send_email");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("skill.yaml"),
            make_skill_yaml("send_email", "network"),
        )
        .unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();

        // 3 user skills + built-in skills (read_file & write_file built-ins
        // skipped because user versions exist)
        assert_eq!(
            manager.registry().count(),
            3 + builtin::BUILTIN_SKILL_NAMES.len() - 2
        );

        // Query by category
        let fs_skills = manager.registry().list_by_category("file-system");
        assert_eq!(fs_skills.len(), 2);

        let net_skills = manager.registry().list_by_category("network");
        // send_email (test) + http_request + web-fetcher (built-in)
        assert_eq!(net_skills.len(), 3);

        // Search
        let results = manager.registry().search("email");
        assert_eq!(results.len(), 1);
        assert!(results.iter().any(|s| s.name == "send_email"));

        // Disable one
        manager.registry().disable("send_email");
        // total - 1 disabled = enabled
        assert_eq!(
            manager.registry().count_enabled(),
            builtin::BUILTIN_SKILL_NAMES.len()
        );

        // Get by name
        let skill = manager.registry().get("read_file").unwrap();
        assert_eq!(skill.category, "file-system");
    }

    #[tokio::test]
    async fn test_module_status_reports_issues() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // One valid skill (loads) + one broken dir (no skill.yaml).
        let valid = skills_dir.join("valid_skill");
        std::fs::create_dir_all(&valid).unwrap();
        std::fs::write(
            valid.join("skill.yaml"),
            make_skill_yaml("valid_skill", "utility"),
        )
        .unwrap();

        let broken = skills_dir.join("broken_skill");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("readme.txt"), "no manifest").unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        let status = manager.module_status();

        // P30 WS1 §3: the broken dir is surfaced as an issue, not silently dropped.
        assert_eq!(status.issues.len(), 1);
        assert_eq!(status.issues[0].kind, ScanIssueKind::MissingManifest);
        assert!(status.has_issues());
        // The valid skill still loads despite the broken neighbour.
        assert!(manager.registry().exists("valid_skill"));
    }
}
