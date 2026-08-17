//! Skill directory scanner — discovers and loads skills from `~/.caspian/skills/`.
//!
//! Uses `tokio::task::JoinSet` with a `Semaphore` to cap parallelism at 32
//! concurrent directory reads, preventing file-handle exhaustion.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::schema::Skill;
use super::validator;

/// Maximum number of skill directories scanned in parallel.
const MAX_CONCURRENCY: usize = 32;

/// Why a skill directory failed to contribute a loadable skill.
///
/// Carried in [`ScanReport::issues`] so the UI can tell the user *exactly*
/// what is missing or broken (P30 §3 — "UI 精确告知缺失"). This is the
/// structured upgrade of the previously-silent `skipped` counter.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanIssueKind {
    /// Directory has no `skill.yaml`.
    MissingManifest,
    /// `skill.yaml` could not be read from disk.
    ReadError,
    /// `skill.yaml` failed to parse as YAML / Skill schema.
    ParseError,
    /// Skill parsed but failed semantic validation.
    ValidationError,
}

/// A single failure encountered while scanning the skills directory.
#[derive(Debug, Clone, Serialize)]
pub struct ScanIssue {
    pub kind: ScanIssueKind,
    /// Filesystem path of the offending directory or manifest (stringified for FFI).
    pub path: String,
    /// Skill name if known (parse got far enough to read `name`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    /// Human-readable reason (error message).
    pub reason: String,
}

/// Structured result of a full skills-directory scan (P30 WS1).
///
/// `skills` are the loadable skills; `issues` are every directory that was
/// skipped, with the *reason* — so resilience is observable instead of a
/// silent `skipped` count. `scanned_dirs` is the number of subdirectories
/// considered (for UI diagnostics).
#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub skills: Vec<Skill>,
    pub issues: Vec<ScanIssue>,
    pub scanned_dirs: usize,
}

impl ScanReport {
    /// An empty report (used before the first scan / when the dir is absent).
    pub fn empty() -> Self {
        Self {
            skills: Vec::new(),
            issues: Vec::new(),
            scanned_dirs: 0,
        }
    }

    /// Whether the scan surfaced any problems.
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }
}

/// Scans the skills directory and loads all valid skills.
pub struct SkillScanner {
    skills_dir: PathBuf,
    max_concurrency: usize,
}

impl SkillScanner {
    /// Create a scanner for the given skills directory.
    pub fn new(skills_dir: &Path) -> Self {
        Self {
            skills_dir: skills_dir.to_path_buf(),
            max_concurrency: MAX_CONCURRENCY,
        }
    }

    /// Create a scanner with a custom concurrency limit.
    pub fn with_concurrency(skills_dir: &Path, max_concurrency: usize) -> Self {
        Self {
            skills_dir: skills_dir.to_path_buf(),
            max_concurrency: max_concurrency.max(1),
        }
    }

    /// Get the skills directory being scanned.
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    /// Scan all subdirectories of the skills directory and load valid skills.
    ///
    /// Returns a [`ScanReport`] instead of a bare `Vec<Skill>`: every directory
    /// that is skipped (no manifest / unreadable / unparseable / invalid) is
    /// recorded as a [`ScanIssue`] with the *reason*, so missing or broken
    /// modules are observable by the UI (P30 §3 — "UI 精确告知缺失") rather
    /// than disappearing into a silent `skipped` counter.
    pub async fn scan(&self) -> ScanReport {
        let mut report = ScanReport::empty();

        // 1. List subdirectories
        let skill_dirs = match self.list_skill_dirs() {
            Ok(dirs) => dirs,
            Err(e) => {
                tracing::warn!(
                    dir = %self.skills_dir.display(),
                    error = %e,
                    "failed to read skills directory"
                );
                return report;
            }
        };

        report.scanned_dirs = skill_dirs.len();

        if skill_dirs.is_empty() {
            tracing::info!(
                dir = %self.skills_dir.display(),
                "no skill directories found"
            );
            return report;
        }

        tracing::info!(
            dir = %self.skills_dir.display(),
            count = skill_dirs.len(),
            concurrency = self.max_concurrency,
            "scanning skill directories"
        );

        // 2. Scan each directory in parallel with bounded concurrency
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let mut join_set = JoinSet::new();

        for dir in skill_dirs {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            join_set.spawn(async move {
                let _permit = permit; // held until task completes
                scan_skill_dir(&dir).await
            });
        }

        // 3. Collect results — both successes and structured failures
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(skill)) => report.skills.push(skill),
                Ok(Err(issue)) => report.issues.push(issue),
                Err(e) => {
                    tracing::error!(error = %e, "skill scan task panicked");
                }
            }
        }

        tracing::info!(
            loaded = report.skills.len(),
            skipped = report.issues.len(),
            "skill scan complete"
        );

        report
    }

    /// List all subdirectories of the skills directory.
    fn list_skill_dirs(&self) -> std::io::Result<Vec<PathBuf>> {
        let mut dirs = Vec::new();

        for entry in std::fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                dirs.push(entry.path());
            }
        }

        Ok(dirs)
    }
}

/// Scan a single skill directory: read `skill.yaml`, parse, validate.
///
/// Returns `Ok(Skill)` on success or `Err(ScanIssue)` on any failure, carrying
/// the *kind* and *reason* so the caller can surface it to the UI (P30 WS1).
async fn scan_skill_dir(dir: &Path) -> Result<Skill, ScanIssue> {
    let manifest_path = dir.join("skill.yaml");

    if !manifest_path.exists() {
        tracing::warn!(dir = %dir.display(), "skill.yaml not found, skipping");
        return Err(ScanIssue {
            kind: ScanIssueKind::MissingManifest,
            path: dir.display().to_string(),
            skill_name: None,
            reason: "directory has no skill.yaml".to_string(),
        });
    }

    // Read the file asynchronously
    let contents = match tokio::fs::read_to_string(&manifest_path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read skill.yaml"
            );
            return Err(ScanIssue {
                kind: ScanIssueKind::ReadError,
                path: manifest_path.display().to_string(),
                skill_name: None,
                reason: e.to_string(),
            });
        }
    };

    // Parse (synchronous — fast CPU-bound operation)
    let mut skill = match Skill::from_yaml_at(&contents, &manifest_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to parse skill.yaml, skipping"
            );
            return Err(ScanIssue {
                kind: ScanIssueKind::ParseError,
                path: manifest_path.display().to_string(),
                skill_name: None,
                reason: e.to_string(),
            });
        }
    };

    // Set the skill directory path
    skill.path = dir.to_path_buf();

    // Validate
    if let Err(e) = validator::validate(&skill) {
        tracing::warn!(
            name = %skill.name,
            path = %dir.display(),
            error = %e,
            "skill validation failed, skipping"
        );
        return Err(ScanIssue {
            kind: ScanIssueKind::ValidationError,
            path: dir.display().to_string(),
            skill_name: Some(skill.name.clone()),
            reason: e.to_string(),
        });
    }

    tracing::info!(
        name = %skill.name,
        path = %dir.display(),
        category = %skill.category,
        "skill loaded successfully"
    );

    Ok(skill)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::skill_template;

    fn make_skill_yaml(name: &str, category: &str) -> &'static str {
        // We can't easily return dynamic strings, so use a helper
        Box::leak(
            format!(
                r#"schema_version: "1.0"
name: "{}"
display_name: "{}"
version: "1.0.0"
description: "A test skill"
category: "{}"

trigger_phrases:
  - "test trigger"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["input"]
  properties:
    input:
      type: "string"

output_schema:
  type: "object"
  required: ["result"]
  properties:
    result:
      type: "string"

permissions:
  fs: []
  network: false
  shell: false

tags:
  - "test"

author: "Test"
license: "MIT"
"#,
                name,
                name.replace('_', " "),
                category
            )
            .into_boxed_str(),
        )
    }

    #[tokio::test]
    async fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let scanner = SkillScanner::new(tmp.path());
        let report = scanner.scan().await;
        assert!(report.skills.is_empty());
        assert_eq!(report.scanned_dirs, 0);
        assert!(!report.has_issues());
    }

    #[tokio::test]
    async fn test_scan_nonexistent_dir() {
        let scanner = SkillScanner::new(Path::new("/nonexistent/skills"));
        let report = scanner.scan().await;
        assert!(report.skills.is_empty());
        assert_eq!(report.scanned_dirs, 0);
    }

    #[tokio::test]
    async fn test_scan_single_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let skill_dir = skills_dir.join("test_skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.yaml"),
            make_skill_yaml("test_skill", "utility"),
        )
        .unwrap();

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;

        assert_eq!(report.skills.len(), 1);
        assert_eq!(report.skills[0].name, "test_skill");
        assert_eq!(report.skills[0].category, "utility");
        assert_eq!(report.skills[0].path, skill_dir);
    }

    #[tokio::test]
    async fn test_scan_multiple_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        for name in ["skill_a", "skill_b", "skill_c"] {
            let dir = skills_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("skill.yaml"), make_skill_yaml(name, "utility")).unwrap();
        }

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;

        assert_eq!(report.skills.len(), 3);
        let names: Vec<&str> = report.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"skill_a"));
        assert!(names.contains(&"skill_b"));
        assert!(names.contains(&"skill_c"));
    }

    #[tokio::test]
    async fn test_scan_skips_invalid_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Valid skill
        let valid_dir = skills_dir.join("valid_skill");
        std::fs::create_dir_all(&valid_dir).unwrap();
        std::fs::write(
            valid_dir.join("skill.yaml"),
            make_skill_yaml("valid_skill", "utility"),
        )
        .unwrap();

        // Invalid skill (missing required field: name)
        let invalid_dir = skills_dir.join("invalid_skill");
        std::fs::create_dir_all(&invalid_dir).unwrap();
        std::fs::write(
            invalid_dir.join("skill.yaml"),
            r#"schema_version: "1.0"
display_name: "No Name"
runtime:
  type: "python"
  entry: "script.py"
"#,
        )
        .unwrap();

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;

        assert_eq!(report.skills.len(), 1);
        assert_eq!(report.skills[0].name, "valid_skill");
        // P30 WS1: broken skill is reported with kind + reason, not silently dropped
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, ScanIssueKind::ParseError);
        assert!(report.issues[0].skill_name.is_none());
        assert!(!report.issues[0].reason.is_empty());
    }

    #[tokio::test]
    async fn test_scan_skips_dir_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Dir with skill.yaml
        let with_manifest = skills_dir.join("has_manifest");
        std::fs::create_dir_all(&with_manifest).unwrap();
        std::fs::write(
            with_manifest.join("skill.yaml"),
            make_skill_yaml("has_manifest", "utility"),
        )
        .unwrap();

        // Dir without skill.yaml
        let no_manifest = skills_dir.join("no_manifest");
        std::fs::create_dir_all(&no_manifest).unwrap();
        std::fs::write(no_manifest.join("readme.txt"), "just a readme").unwrap();

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;

        assert_eq!(report.skills.len(), 1);
        assert_eq!(report.skills[0].name, "has_manifest");
        // P30 WS1: the manifest-less dir is reported as MissingManifest
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, ScanIssueKind::MissingManifest);
        assert!(report.issues[0].skill_name.is_none());
    }

    #[tokio::test]
    async fn test_scan_reports_validation_error_with_name() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Parses OK (has `name` + `runtime`) but fails semantic validation
        // (empty runtime.entry -> validator error).
        let bad_dir = skills_dir.join("bad_validation");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(
            bad_dir.join("skill.yaml"),
            r#"schema_version: "1.0"
name: "bad_validation"
display_name: "Bad"
runtime:
  type: "python"
  entry: ""
"#,
        )
        .unwrap();

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;

        assert!(report.skills.is_empty());
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, ScanIssueKind::ValidationError);
        assert_eq!(report.issues[0].skill_name.as_deref(), Some("bad_validation"));
    }

    #[tokio::test]
    async fn test_scan_skips_files_in_skills_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // A file (not directory) in the skills directory — should be ignored
        std::fs::write(skills_dir.join("random_file.txt"), "hello").unwrap();

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;
        assert!(report.skills.is_empty());
        // A loose file is not a skill dir, so it must not surface as an issue.
        assert!(!report.has_issues());
    }

    #[tokio::test]
    async fn test_scan_with_template_created_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Create skills using the template generator
        skill_template::create_skill_template(
            &skills_dir.join("templated_skill"),
            "templated_skill",
        )
        .unwrap();

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;

        assert_eq!(report.skills.len(), 1);
        assert_eq!(report.skills[0].name, "templated_skill");
        assert_eq!(report.skills[0].category, "utility");
    }

    #[tokio::test]
    async fn test_scan_ten_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        for i in 0..10 {
            let name = format!("skill_{i:02}");
            let dir = skills_dir.join(&name);
            std::fs::create_dir_all(&dir).unwrap();

            let yaml = format!(
                r#"schema_version: "1.0"
name: "{name}"
display_name: "Skill {i}"
version: "1.0.0"
description: "Test skill {i}"
category: "test"

trigger_phrases:
  - "test {i}"

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
            );
            std::fs::write(dir.join("skill.yaml"), yaml).unwrap();
        }

        let scanner = SkillScanner::new(&skills_dir);
        let report = scanner.scan().await;

        assert_eq!(report.skills.len(), 10);
    }

    #[tokio::test]
    async fn test_custom_concurrency_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Create 3 skills
        for name in ["a", "b", "c"] {
            let dir = skills_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("skill.yaml"), make_skill_yaml(name, "test")).unwrap();
        }

        // Use concurrency limit of 1 (sequential)
        let scanner = SkillScanner::with_concurrency(&skills_dir, 1);
        let report = scanner.scan().await;

        assert_eq!(report.skills.len(), 3);
    }

    #[test]
    fn test_list_skill_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::create_dir_all(skills_dir.join("skill_a")).unwrap();
        std::fs::create_dir_all(skills_dir.join("skill_b")).unwrap();
        std::fs::write(skills_dir.join("file.txt"), "hello").unwrap();

        let scanner = SkillScanner::new(&skills_dir);
        let dirs = scanner.list_skill_dirs().unwrap();

        assert_eq!(dirs.len(), 2);
    }

    #[test]
    fn test_skills_dir_accessor() {
        let path = Path::new("/custom/skills");
        let scanner = SkillScanner::new(path);
        assert_eq!(scanner.skills_dir(), path);
    }
}
