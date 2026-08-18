//! Skill validation logic — checks required fields, range constraints,
//! and schema version compatibility.

use super::schema::{Skill, SkillRuntimeType, SKILL_SCHEMA_VERSION};
use crate::types::{SkillError, SkillResult};

/// Result of skill validation: errors block registration, warnings are advisory.
#[derive(Debug, Clone, Default)]
pub struct SkillValidationResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl SkillValidationResult {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn add_error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    pub fn add_warning(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

/// Validate a skill and return `Ok(())` if it passes, or an error if it fails.
///
/// Warnings are logged but do not block registration.
pub fn validate(skill: &Skill) -> SkillResult<()> {
    let result = validate_with_warnings(skill)?;
    if !result.is_valid() {
        return Err(SkillError::ValidationError {
            path: skill.path.display().to_string(),
            errors: result.errors.join("; "),
        });
    }
    if !result.warnings.is_empty() {
        tracing::warn!(
            skill = %skill.name,
            warnings = ?result.warnings,
            "skill validation passed with warnings"
        );
    }
    Ok(())
}

/// Validate a skill and return the full result (errors + warnings).
pub fn validate_with_warnings(skill: &Skill) -> SkillResult<SkillValidationResult> {
    let mut result = SkillValidationResult::default();

    // 1. Required string fields
    if skill.name.is_empty() {
        result.add_error("name must not be empty");
    }

    if skill.runtime.entry.is_empty() {
        result.add_error("runtime.entry must not be empty");
    }

    if skill.category.is_empty() {
        result.add_error("category must not be empty");
    }

    // 2. Name format — must be a valid identifier (lowercase, underscores, hyphens)
    if !skill.name.is_empty() && !is_valid_skill_name(&skill.name) {
        result.add_error(format!(
            "name `{}` is not a valid identifier (use lowercase letters, digits, hyphens, underscores)",
            skill.name
        ));
    }

    // 3. Version format (semver-like)
    if !skill.version.is_empty() && !is_valid_version(&skill.version) {
        result.add_warning(format!(
            "version `{}` does not follow semver format (e.g. 1.0.0)",
            skill.version
        ));
    }

    // 4. Trigger phrases
    if skill.trigger_phrases.is_empty() {
        result.add_warning("trigger_phrases is empty — skill will not match in semantic routing");
    } else {
        for (i, phrase) in skill.trigger_phrases.iter().enumerate() {
            if phrase.trim().is_empty() {
                result.add_warning(format!("trigger_phrases[{i}] is empty"));
            }
        }
    }

    // 5. Display name defaults
    if skill.display_name.is_empty() {
        result.add_warning("display_name is empty, will use name as display name");
    }

    // 6. Description
    if skill.description.is_empty() {
        result.add_warning("description is empty");
    }

    // 7. Runtime checks
    match skill.runtime.runtime_type {
        SkillRuntimeType::Python => {
            if !skill.runtime.entry.ends_with(".py") {
                result.add_warning(format!(
                    "runtime.entry `{}` does not end with .py for python runtime",
                    skill.runtime.entry
                ));
            }
        }
        SkillRuntimeType::Javascript => {
            if !skill.runtime.entry.ends_with(".js") && !skill.runtime.entry.ends_with(".mjs") {
                result.add_warning(format!(
                    "runtime.entry `{}` does not end with .js/.mjs for javascript runtime",
                    skill.runtime.entry
                ));
            }
        }
        SkillRuntimeType::Shell => {
            if !skill.runtime.entry.ends_with(".sh") {
                result.add_warning(format!(
                    "runtime.entry `{}` does not end with .sh for shell runtime",
                    skill.runtime.entry
                ));
            }
        }
        SkillRuntimeType::RustBinary => {
            // No extension check for binary
        }
    }

    if skill.runtime.timeout == 0 {
        result.add_warning("runtime.timeout is 0, will use default 30s");
    }
    if skill.runtime.timeout > 600 {
        result.add_warning(format!(
            "runtime.timeout {}s is very large (max recommended: 600s)",
            skill.runtime.timeout
        ));
    }
    if skill.runtime.memory_limit_mb == 0 {
        result.add_warning("runtime.memory_limit_mb is 0, will use default 256MB");
    }
    if skill.runtime.memory_limit_mb > 2048 {
        result.add_warning(format!(
            "runtime.memory_limit_mb {}MB is very large (max recommended: 2048MB)",
            skill.runtime.memory_limit_mb
        ));
    }

    // 8. Schema version compatibility
    if skill.schema_version != SKILL_SCHEMA_VERSION {
        result.add_warning(format!(
            "schema_version `{}` differs from current `{}`",
            skill.schema_version, SKILL_SCHEMA_VERSION
        ));
    }

    // 9. Input/output schema basic check
    if !skill.input_schema.is_object() {
        result.add_warning("input_schema is not a JSON object");
    }
    if !skill.output_schema.is_object() {
        result.add_warning("output_schema is not a JSON object");
    }

    // 10. Permissions sanity
    if skill.permissions.network && skill.permissions.shell {
        result.add_warning("skill has both network and shell permissions — high risk");
    }

    Ok(result)
}

/// Check if a skill name is a valid identifier.
fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
}

/// Check if a version string follows semver-like format (major.minor.patch).
fn is_valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u32>().is_ok())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::{Skill, SkillRuntime, SkillRuntimeType};
    use std::path::PathBuf;

    fn make_valid_skill() -> Skill {
        Skill {
            mcp: None,
            schema_version: SKILL_SCHEMA_VERSION.to_string(),
            name: "read_file".to_string(),
            display_name: "Read File".to_string(),
            version: "1.0.0".to_string(),
            description: "Reads a file".to_string(),
            category: "file-system".to_string(),
            trigger_phrases: vec!["read file".to_string()],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Python,
                entry: "script.py".to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            permissions: Default::default(),
            tags: vec!["file".to_string()],
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from("/skills/read_file"),
        }
    }

    #[test]
    fn test_valid_skill_passes() {
        let skill = make_valid_skill();
        assert!(validate(&skill).is_ok());
    }

    #[test]
    fn test_empty_name_fails() {
        let mut skill = make_valid_skill();
        skill.name = String::new();
        assert!(validate(&skill).is_err());
    }

    #[test]
    fn test_empty_category_fails() {
        let mut skill = make_valid_skill();
        skill.category = String::new();
        assert!(validate(&skill).is_err());
    }

    #[test]
    fn test_empty_entry_fails() {
        let mut skill = make_valid_skill();
        skill.runtime.entry = String::new();
        assert!(validate(&skill).is_err());
    }

    #[test]
    fn test_invalid_name_fails() {
        let mut skill = make_valid_skill();
        skill.name = "Read File!".to_string();
        assert!(validate(&skill).is_err());
    }

    #[test]
    fn test_empty_trigger_phrases_warns() {
        let mut skill = make_valid_skill();
        skill.trigger_phrases = vec![];
        let result = validate_with_warnings(&skill).unwrap();
        assert!(result.is_valid());
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("trigger_phrases")));
    }

    #[test]
    fn test_missing_display_name_warns() {
        let mut skill = make_valid_skill();
        skill.display_name = String::new();
        let result = validate_with_warnings(&skill).unwrap();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("display_name")));
    }

    #[test]
    fn test_python_extension_warning() {
        let mut skill = make_valid_skill();
        skill.runtime.entry = "script.txt".to_string();
        let result = validate_with_warnings(&skill).unwrap();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains(".py")));
    }

    #[test]
    fn test_large_timeout_warning() {
        let mut skill = make_valid_skill();
        skill.runtime.timeout = 999;
        let result = validate_with_warnings(&skill).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("very large")));
    }

    #[test]
    fn test_schema_version_mismatch_warning() {
        let mut skill = make_valid_skill();
        skill.schema_version = "0.9".to_string();
        let result = validate_with_warnings(&skill).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("schema_version")));
    }

    #[test]
    fn test_network_and_shell_warning() {
        let mut skill = make_valid_skill();
        skill.permissions.network = true;
        skill.permissions.shell = true;
        let result = validate_with_warnings(&skill).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("high risk")));
    }

    #[test]
    fn test_valid_version_formats() {
        assert!(is_valid_version("1.0.0"));
        assert!(is_valid_version("2.1"));
        assert!(is_valid_version("0.0.1"));
        assert!(!is_valid_version("1"));
        assert!(!is_valid_version("v1.0.0"));
        assert!(!is_valid_version("1.0.0-beta"));
    }

    #[test]
    fn test_valid_skill_names() {
        assert!(is_valid_skill_name("read_file"));
        assert!(is_valid_skill_name("read-file"));
        assert!(is_valid_skill_name("skill123"));
        assert!(is_valid_skill_name("_private"));
        assert!(!is_valid_skill_name(""));
        assert!(!is_valid_skill_name("ReadFile"));
        assert!(!is_valid_skill_name("read file"));
        assert!(!is_valid_skill_name("123skill"));
        assert!(!is_valid_skill_name("read$file"));
    }
}
