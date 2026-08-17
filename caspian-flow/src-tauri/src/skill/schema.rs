//! Skill data model — the core struct matching `skill.yaml` schema.
//!
//! Each skill lives in `~/.caspian/skills/<skill-name>/skill.yaml` and is
//! deserialized into [`Skill`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::types::{SkillError, SkillResult};

/// Current skill.yaml schema version.
pub const SKILL_SCHEMA_VERSION: &str = "1.0";

// ---------------------------------------------------------------------------
// Runtime types
// ---------------------------------------------------------------------------

/// The execution runtime for a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SkillRuntimeType {
    #[default]
    Python,
    Javascript,
    RustBinary,
    Shell,
}

impl std::fmt::Display for SkillRuntimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Python => write!(f, "python"),
            Self::Javascript => write!(f, "javascript"),
            Self::RustBinary => write!(f, "rust_binary"),
            Self::Shell => write!(f, "shell"),
        }
    }
}

/// Runtime configuration for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRuntime {
    #[serde(rename = "type")]
    pub runtime_type: SkillRuntimeType,

    pub entry: String,

    #[serde(default = "default_timeout")]
    pub timeout: u64,

    #[serde(default = "default_memory_limit")]
    pub memory_limit_mb: u32,
}

fn default_timeout() -> u64 {
    30
}

fn default_memory_limit() -> u32 {
    256
}

impl Default for SkillRuntime {
    fn default() -> Self {
        Self {
            runtime_type: SkillRuntimeType::default(),
            entry: String::new(),
            timeout: default_timeout(),
            memory_limit_mb: default_memory_limit(),
        }
    }
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

/// Filesystem permission rule for a skill.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FsPermission {
    #[serde(default)]
    pub read: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Vec<String>>,
}

/// Permission declaration for a skill.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillPermissions {
    #[serde(default)]
    pub fs: Vec<FsPermission>,

    #[serde(default)]
    pub network: bool,

    #[serde(default)]
    pub shell: bool,
}

// ---------------------------------------------------------------------------
// Skill struct
// ---------------------------------------------------------------------------

/// A loaded skill with all metadata from `skill.yaml`.
///
/// Fields `enabled` and `path` are runtime state, not stored in the YAML file.
/// `enabled` defaults to `true`; `path` is set by the scanner after parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,

    pub name: String,

    #[serde(default)]
    pub display_name: String,

    #[serde(default)]
    pub version: String,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub category: String,

    #[serde(default)]
    pub trigger_phrases: Vec<String>,

    #[serde(default)]
    pub runtime: SkillRuntime,

    #[serde(default = "default_empty_object")]
    pub input_schema: serde_json::Value,

    #[serde(default = "default_empty_object")]
    pub output_schema: serde_json::Value,

    #[serde(default)]
    pub permissions: SkillPermissions,

    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub author: String,

    #[serde(default)]
    pub license: String,

    // --- Runtime state (not in skill.yaml) ---
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(skip)]
    pub path: PathBuf,

    /// Optional binding to a tool exposed by an external MCP (Model Context
    /// Protocol) server. When present, the skill is executed by calling that
    /// server's tool over a JSON-RPC 2.0 stdio session (`skill::mcp`), not by
    /// running a local entry script. External code still runs inside the P32
    /// sandbox (the server is launched with the sandbox's working directory and
    /// policy env). Absent for normal local skills.
    #[serde(default)]
    pub mcp: Option<McpRef>,
}

/// Binding to a tool on an external MCP server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpRef {
    /// How to launch the MCP server: `[program, ..args]`.
    pub server_command: Vec<String>,
    /// The tool name to invoke on that server.
    pub tool: String,
}

fn default_schema_version() -> String {
    SKILL_SCHEMA_VERSION.to_string()
}

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
}

fn default_true() -> bool {
    true
}

impl Skill {
    /// Parse a skill from a YAML string.
    pub fn from_yaml(yaml: &str) -> SkillResult<Self> {
        if yaml.trim().is_empty() {
            return Err(SkillError::ParseError {
                path: "(inline)".to_string(),
                reason: "empty skill.yaml".to_string(),
            });
        }

        serde_yaml::from_str::<Self>(yaml).map_err(|e| SkillError::ParseError {
            path: "(inline)".to_string(),
            reason: e.to_string(),
        })
    }

    /// Parse a skill from a YAML string, using `manifest_path` for error context.
    pub fn from_yaml_at(yaml: &str, manifest_path: &Path) -> SkillResult<Self> {
        if yaml.trim().is_empty() {
            return Err(SkillError::ParseError {
                path: manifest_path.display().to_string(),
                reason: "empty skill.yaml".to_string(),
            });
        }

        serde_yaml::from_str::<Self>(yaml).map_err(|e| SkillError::ParseError {
            path: manifest_path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Load a skill from a `skill.yaml` file.
    pub fn load(manifest_path: &Path) -> SkillResult<Self> {
        let contents =
            std::fs::read_to_string(manifest_path).map_err(|e| SkillError::ParseError {
                path: manifest_path.display().to_string(),
                reason: e.to_string(),
            })?;

        let mut skill = Self::from_yaml_at(&contents, manifest_path)?;
        // Set the skill directory (parent of skill.yaml) as the path
        if let Some(parent) = manifest_path.parent() {
            skill.path = parent.to_path_buf();
        }
        Ok(skill)
    }

    /// Serialize the skill to a YAML string (without runtime fields).
    pub fn to_yaml(&self) -> SkillResult<String> {
        serde_yaml::to_string(self).map_err(|e| SkillError::ParseError {
            path: self.path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Get the skill directory path.
    pub fn dir(&self) -> &Path {
        &self.path
    }

    /// Get the path to the skill's entry script.
    pub fn entry_path(&self) -> PathBuf {
        self.path.join(&self.runtime.entry)
    }

    /// Get the examples directory path.
    pub fn examples_dir(&self) -> PathBuf {
        self.path.join("examples")
    }

    /// Get the assets directory path.
    pub fn assets_dir(&self) -> PathBuf {
        self.path.join("assets")
    }

    /// Check if the skill has a specific tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"schema_version: "1.0"
name: "read_file"
display_name: "读取文件"
version: "1.0.0"
description: "读取本地文本文件的内容"
category: "file-system"

trigger_phrases:
  - "读取文件"
  - "打开文件"
  - "read file"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["path"]
  properties:
    path:
      type: "string"
      description: "文件路径"

output_schema:
  type: "object"
  required: ["content"]
  properties:
    content:
      type: "string"

permissions:
  fs:
    - read: ["~/.caspian", "{workspace}"]
  network: false
  shell: false

tags:
  - "file"
  - "read"

author: "Caspian Team"
license: "MIT"
"#;

    #[test]
    fn test_parse_full_skill() {
        let skill = Skill::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(skill.schema_version, "1.0");
        assert_eq!(skill.name, "read_file");
        assert_eq!(skill.display_name, "读取文件");
        assert_eq!(skill.version, "1.0.0");
        assert_eq!(skill.category, "file-system");
        assert_eq!(skill.trigger_phrases.len(), 3);
        assert_eq!(skill.runtime.runtime_type, SkillRuntimeType::Python);
        assert_eq!(skill.runtime.entry, "script.py");
        assert_eq!(skill.runtime.timeout, 30);
        assert_eq!(skill.runtime.memory_limit_mb, 256);
        assert_eq!(skill.tags, vec!["file", "read"]);
        assert_eq!(skill.author, "Caspian Team");
        assert_eq!(skill.license, "MIT");
        assert!(skill.enabled); // defaults to true
        assert!(skill.path.as_os_str().is_empty()); // not set during parse
    }

    #[test]
    fn test_parse_with_defaults() {
        let yaml = r#"
name: "minimal_skill"
runtime:
  type: "shell"
  entry: "run.sh"
"#;
        let skill = Skill::from_yaml(yaml).unwrap();
        assert_eq!(skill.name, "minimal_skill");
        assert_eq!(skill.schema_version, "1.0");
        assert!(skill.display_name.is_empty());
        assert!(skill.description.is_empty());
        assert!(skill.category.is_empty());
        assert!(skill.trigger_phrases.is_empty());
        assert_eq!(skill.runtime.runtime_type, SkillRuntimeType::Shell);
        assert_eq!(skill.runtime.entry, "run.sh");
        assert_eq!(skill.runtime.timeout, 30); // default
        assert_eq!(skill.runtime.memory_limit_mb, 256); // default
        assert!(skill.enabled);
    }

    #[test]
    fn test_roundtrip_yaml() {
        let skill = Skill::from_yaml(SAMPLE_YAML).unwrap();
        let yaml = skill.to_yaml().unwrap();
        let reparsed = Skill::from_yaml(&yaml).unwrap();
        assert_eq!(skill.name, reparsed.name);
        assert_eq!(skill.display_name, reparsed.display_name);
        assert_eq!(skill.runtime.runtime_type, reparsed.runtime.runtime_type);
        assert_eq!(skill.tags, reparsed.tags);
    }

    #[test]
    fn test_empty_yaml_errors() {
        let result = Skill::from_yaml("");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_name_errors() {
        let yaml = r#"
display_name: "No Name"
runtime:
  type: "python"
  entry: "script.py"
"#;
        let result = Skill::from_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_runtime_type_serialization() {
        let yaml = r#"
name: "test"
runtime:
  type: "rust_binary"
  entry: "./binary"
"#;
        let skill = Skill::from_yaml(yaml).unwrap();
        assert_eq!(skill.runtime.runtime_type, SkillRuntimeType::RustBinary);
    }

    #[test]
    fn test_permissions_parsing() {
        let skill = Skill::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(skill.permissions.fs.len(), 1);
        assert_eq!(
            skill.permissions.fs[0].read,
            vec!["~/.caspian", "{workspace}"]
        );
        assert!(!skill.permissions.network);
        assert!(!skill.permissions.shell);
    }

    #[test]
    fn test_json_schema_parsing() {
        let skill = Skill::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(skill.input_schema["type"], "object");
        assert_eq!(skill.input_schema["required"][0], "path");
        assert_eq!(skill.output_schema["type"], "object");
    }

    #[test]
    fn test_load_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("test_skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("skill.yaml"), SAMPLE_YAML).unwrap();

        let skill = Skill::load(&skill_dir.join("skill.yaml")).unwrap();
        assert_eq!(skill.name, "read_file");
        assert_eq!(skill.path, skill_dir);
    }

    #[test]
    fn test_entry_path() {
        let mut skill = Skill::from_yaml(SAMPLE_YAML).unwrap();
        skill.path = PathBuf::from("/skills/read_file");
        assert_eq!(
            skill.entry_path(),
            PathBuf::from("/skills/read_file/script.py")
        );
    }

    #[test]
    fn test_has_tag() {
        let skill = Skill::from_yaml(SAMPLE_YAML).unwrap();
        assert!(skill.has_tag("file"));
        assert!(skill.has_tag("read"));
        assert!(!skill.has_tag("network"));
    }

    #[test]
    fn test_runtime_type_display() {
        assert_eq!(SkillRuntimeType::Python.to_string(), "python");
        assert_eq!(SkillRuntimeType::Javascript.to_string(), "javascript");
        assert_eq!(SkillRuntimeType::RustBinary.to_string(), "rust_binary");
        assert_eq!(SkillRuntimeType::Shell.to_string(), "shell");
    }
}
