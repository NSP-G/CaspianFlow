//! Workflow data model — the core struct matching `workflow.yaml` schema.
//!
//! Each workflow lives in `~/.caspian/workflows/<workflow-name>/workflow.yaml`
//! and is deserialized into [`Workflow`].
//!
//! ## Workflow YAML structure
//!
//! ```yaml
//! schema_version: "1.0"
//! name: "process_document"
//! display_name: "Process Document"
//! version: "1.0.0"
//! description: "Read, summarize, and save a document"
//! category: "document"
//!
//! trigger_phrases:
//!   - "process document"
//!
//! variables:
//!   - name: "input_path"
//!     type: "string"
//!     description: "Path to the document"
//!     required: true
//!
//! steps:
//!   - id: "read"
//!     skill: "read_file"
//!     input:
//!       path: "${variables.input_path}"
//!     output: "content"
//!
//!   - id: "summarize"
//!     skill: "summarize_text"
//!     input:
//!       text: "${steps.read.output.content}"
//!       max_length: 200
//!     depends_on: ["read"]
//!
//! error_handling:
//!   on_step_failure: "abort"
//!   max_retries: 2
//!   retry_delay_ms: 1000
//!
//! timeout: 300
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::types::{WorkflowError, WorkflowResult};

/// Current workflow.yaml schema version.
pub const WORKFLOW_SCHEMA_VERSION: &str = "1.0";

// ---------------------------------------------------------------------------
// Error handling strategy
// ---------------------------------------------------------------------------

/// Strategy for handling step failures during workflow execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorHandlingStrategy {
    /// Abort the entire workflow on first step failure (default).
    #[default]
    Abort,
    /// Continue executing independent steps; skip dependent steps.
    Continue,
    /// Retry the failed step up to `max_retries` times.
    Retry,
}

impl std::fmt::Display for ErrorHandlingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Abort => write!(f, "abort"),
            Self::Continue => write!(f, "continue"),
            Self::Retry => write!(f, "retry"),
        }
    }
}

// ---------------------------------------------------------------------------
// Workflow variable
// ---------------------------------------------------------------------------

/// A workflow-level variable definition.
///
/// Variables are inputs that the user provides when starting a workflow.
/// They can be referenced in step inputs via `${variables.<name>}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowVariable {
    /// Variable name (used in `${variables.<name>}` expressions).
    pub name: String,

    /// Variable type: "string", "number", "boolean", "object".
    #[serde(default = "default_var_type")]
    #[serde(rename = "type")]
    pub var_type: String,

    /// Human-readable description.
    #[serde(default)]
    pub description: String,

    /// Whether this variable must be provided by the user.
    #[serde(default)]
    pub required: bool,

    /// Default value if not provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

fn default_var_type() -> String {
    "string".to_string()
}

// ---------------------------------------------------------------------------
// Workflow step
// ---------------------------------------------------------------------------

/// A single step in a workflow — corresponds to one skill invocation.
///
/// Steps form a DAG via `depends_on`. A step can only execute after all
/// its dependencies have completed successfully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Unique step identifier within the workflow.
    pub id: String,

    /// Name of the skill to invoke.
    pub skill: String,

    /// Input parameters for the skill.
    ///
    /// Values can contain template expressions like `${variables.xxx}`
    /// or `${steps.<id>.output.<field>}`.
    #[serde(default = "default_empty_object")]
    pub input: serde_json::Value,

    /// Output variable name — the skill's output JSON is stored under
    /// `steps.<id>.output` for use by later steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,

    /// Step IDs that must complete before this step can run.
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Optional condition expression — if it evaluates to false, the
    /// step is skipped.
    ///
    /// Example: `${steps.check.output.size} > 1000`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,

    /// Per-step timeout override (seconds). If not set, uses workflow timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Per-step retry count override. If not set, uses workflow error_handling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<usize>,

    /// Variable assignments evaluated after this step completes (P18).
    ///
    /// Each entry maps a variable name to a template expression resolved
    /// against the execution context. Results are written to
    /// `${variables.<name>}` for downstream steps. Substitution only — no
    /// arithmetic (per P18 fork ①).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vars: Option<HashMap<String, String>>,

    /// Iterate this step once per element of the referenced collection (P18).
    ///
    /// The template must resolve to a JSON array; each element is bound to
    /// `as_var` (default `item`) and the step's skill executes per element.
    /// Per-element outputs are collected into a single array stored under
    /// `steps.<id>.output`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterate: Option<String>,

    /// Loop variable name for `iterate` (default: `item`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_var: Option<String>,
}

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
}

// ---------------------------------------------------------------------------
// Error handling config
// ---------------------------------------------------------------------------

/// Configuration for workflow error handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowErrorHandling {
    /// Strategy when a step fails.
    #[serde(default)]
    pub on_step_failure: ErrorHandlingStrategy,

    /// Maximum retry attempts (for Retry strategy).
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,

    /// Delay between retries in milliseconds.
    #[serde(default = "default_retry_delay")]
    pub retry_delay_ms: u64,
}

fn default_max_retries() -> usize {
    2
}

fn default_retry_delay() -> u64 {
    1000
}

impl Default for WorkflowErrorHandling {
    fn default() -> Self {
        Self {
            on_step_failure: ErrorHandlingStrategy::default(),
            max_retries: default_max_retries(),
            retry_delay_ms: default_retry_delay(),
        }
    }
}

// ---------------------------------------------------------------------------
// Early-termination condition (P18)
// ---------------------------------------------------------------------------

/// Early-termination condition for a workflow (P18).
///
/// Checked after every step; when `if_expr` evaluates to true the run stops
/// intentionally (status `Terminated`) and returns the outputs gathered so far.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndCondition {
    /// Boolean expression, e.g. `${steps.result.output.done} == true`.
    /// Serialized as `if` in YAML (`if_expr` would be a reserved word there).
    #[serde(rename = "if")]
    pub if_expr: String,
}

// ---------------------------------------------------------------------------
// Workflow struct
// ---------------------------------------------------------------------------

/// A loaded workflow with all metadata from `workflow.yaml`.
///
/// Fields `enabled` and `path` are runtime state, not stored in the YAML file.
/// `enabled` defaults to `true`; `path` is set by the scanner after parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
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
    pub variables: Vec<WorkflowVariable>,

    #[serde(default)]
    pub steps: Vec<WorkflowStep>,

    #[serde(default)]
    pub error_handling: WorkflowErrorHandling,

    /// Early-termination condition (P18). If it evaluates to true after any
    /// step, the workflow stops and returns the outputs collected so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<EndCondition>,

    /// Total workflow timeout in seconds (default: 300).
    #[serde(default = "default_workflow_timeout")]
    pub timeout: u64,

    /// Max concurrent steps (P19). `None` ⇒ default (4). `1` ⇒ sequential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<usize>,

    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub author: String,

    #[serde(default)]
    pub license: String,

    // --- Runtime state (not in workflow.yaml) ---
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(skip)]
    pub path: PathBuf,
}

fn default_schema_version() -> String {
    WORKFLOW_SCHEMA_VERSION.to_string()
}

fn default_workflow_timeout() -> u64 {
    300
}

fn default_true() -> bool {
    true
}

impl Workflow {
    /// Parse a workflow from a YAML string.
    pub fn from_yaml(yaml: &str) -> WorkflowResult<Self> {
        if yaml.trim().is_empty() {
            return Err(WorkflowError::ParseError {
                path: "(inline)".to_string(),
                reason: "empty workflow.yaml".to_string(),
            });
        }

        serde_yaml::from_str::<Self>(yaml).map_err(|e| WorkflowError::ParseError {
            path: "(inline)".to_string(),
            reason: e.to_string(),
        })
    }

    /// Parse a workflow from a YAML string, using `manifest_path` for error context.
    pub fn from_yaml_at(yaml: &str, manifest_path: &Path) -> WorkflowResult<Self> {
        if yaml.trim().is_empty() {
            return Err(WorkflowError::ParseError {
                path: manifest_path.display().to_string(),
                reason: "empty workflow.yaml".to_string(),
            });
        }

        serde_yaml::from_str::<Self>(yaml).map_err(|e| WorkflowError::ParseError {
            path: manifest_path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Load a workflow from a `workflow.yaml` file.
    pub fn load(manifest_path: &Path) -> WorkflowResult<Self> {
        let contents =
            std::fs::read_to_string(manifest_path).map_err(|e| WorkflowError::ParseError {
                path: manifest_path.display().to_string(),
                reason: e.to_string(),
            })?;

        let mut workflow = Self::from_yaml_at(&contents, manifest_path)?;
        if let Some(parent) = manifest_path.parent() {
            workflow.path = parent.to_path_buf();
        }
        Ok(workflow)
    }

    /// Serialize the workflow to a YAML string.
    pub fn to_yaml(&self) -> WorkflowResult<String> {
        serde_yaml::to_string(self).map_err(|e| WorkflowError::ParseError {
            path: self.path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Get the workflow directory path.
    pub fn dir(&self) -> &Path {
        &self.path
    }

    /// Find a step by ID.
    pub fn get_step(&self, step_id: &str) -> Option<&WorkflowStep> {
        self.steps.iter().find(|s| s.id == step_id)
    }

    /// Get all step IDs.
    pub fn step_ids(&self) -> Vec<&str> {
        self.steps.iter().map(|s| s.id.as_str()).collect()
    }

    /// Get all required variable names.
    pub fn required_variables(&self) -> Vec<&str> {
        self.variables
            .iter()
            .filter(|v| v.required)
            .map(|v| v.name.as_str())
            .collect()
    }

    /// Check if the workflow has a specific tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Get the number of steps.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Get entry steps (steps with no dependencies).
    pub fn entry_steps(&self) -> Vec<&WorkflowStep> {
        self.steps
            .iter()
            .filter(|s| s.depends_on.is_empty())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"schema_version: "1.0"
name: "process_document"
display_name: "Process Document"
version: "1.0.0"
description: "Read, summarize, and save a document"
category: "document"

trigger_phrases:
  - "process document"
  - "处理文档"

variables:
  - name: "input_path"
    type: "string"
    description: "Path to the document"
    required: true
  - name: "max_length"
    type: "number"
    description: "Max summary length"
    required: false
    default: 200

steps:
  - id: "read"
    skill: "read_file"
    input:
      path: "${variables.input_path}"
    output: "content"

  - id: "summarize"
    skill: "summarize_text"
    input:
      text: "${steps.read.output.content}"
      max_length: "${variables.max_length}"
    output: "summary"
    depends_on: ["read"]

  - id: "save"
    skill: "write_file"
    input:
      path: "${variables.input_path}.summary.txt"
      content: "${steps.summarize.output.summary}"
    output: "result"
    depends_on: ["summarize"]
    condition: "${steps.summarize.output.summary_length} > 0"

error_handling:
  on_step_failure: "retry"
  max_retries: 3
  retry_delay_ms: 500

timeout: 300

tags:
  - "document"
  - "pipeline"

author: "Caspian Team"
license: "MIT"
"#;

    #[test]
    fn test_parse_full_workflow() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(wf.schema_version, "1.0");
        assert_eq!(wf.name, "process_document");
        assert_eq!(wf.display_name, "Process Document");
        assert_eq!(wf.version, "1.0.0");
        assert_eq!(wf.category, "document");
        assert_eq!(wf.trigger_phrases.len(), 2);
        assert_eq!(wf.variables.len(), 2);
        assert_eq!(wf.steps.len(), 3);
        assert_eq!(wf.timeout, 300);
        assert_eq!(wf.tags, vec!["document", "pipeline"]);
        assert!(wf.enabled);
    }

    #[test]
    fn test_parse_variables() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(wf.variables[0].name, "input_path");
        assert_eq!(wf.variables[0].var_type, "string");
        assert!(wf.variables[0].required);
        assert!(wf.variables[0].default.is_none());

        assert_eq!(wf.variables[1].name, "max_length");
        assert_eq!(wf.variables[1].var_type, "number");
        assert!(!wf.variables[1].required);
        assert_eq!(wf.variables[1].default, Some(serde_json::json!(200)));
    }

    #[test]
    fn test_parse_steps() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();

        let read = wf.get_step("read").unwrap();
        assert_eq!(read.skill, "read_file");
        assert!(read.depends_on.is_empty());
        assert!(read.condition.is_none());

        let summarize = wf.get_step("summarize").unwrap();
        assert_eq!(summarize.skill, "summarize_text");
        assert_eq!(summarize.depends_on, vec!["read"]);

        let save = wf.get_step("save").unwrap();
        assert_eq!(save.depends_on, vec!["summarize"]);
        assert!(save.condition.is_some());
        assert_eq!(save.timeout, None);
    }

    #[test]
    fn test_parse_error_handling() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(
            wf.error_handling.on_step_failure,
            ErrorHandlingStrategy::Retry
        );
        assert_eq!(wf.error_handling.max_retries, 3);
        assert_eq!(wf.error_handling.retry_delay_ms, 500);
    }

    #[test]
    fn test_parse_with_defaults() {
        let yaml = r#"
name: "minimal"
steps:
  - id: "step1"
    skill: "read_file"
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        assert_eq!(wf.name, "minimal");
        assert_eq!(wf.schema_version, "1.0");
        assert!(wf.display_name.is_empty());
        assert!(wf.variables.is_empty());
        assert_eq!(wf.timeout, 300);
        assert_eq!(
            wf.error_handling.on_step_failure,
            ErrorHandlingStrategy::Abort
        );
        assert_eq!(wf.error_handling.max_retries, 2);
        assert!(wf.enabled);
    }

    #[test]
    fn test_roundtrip_yaml() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        let yaml = wf.to_yaml().unwrap();
        let reparsed = Workflow::from_yaml(&yaml).unwrap();
        assert_eq!(wf.name, reparsed.name);
        assert_eq!(wf.steps.len(), reparsed.steps.len());
        assert_eq!(wf.variables.len(), reparsed.variables.len());
    }

    #[test]
    fn test_empty_yaml_errors() {
        let result = Workflow::from_yaml("");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_name_errors() {
        let yaml = r#"
steps:
  - id: "step1"
    skill: "read_file"
"#;
        let result = Workflow::from_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let wf_dir = tmp.path().join("test_workflow");
        std::fs::create_dir_all(&wf_dir).unwrap();
        std::fs::write(wf_dir.join("workflow.yaml"), SAMPLE_YAML).unwrap();

        let wf = Workflow::load(&wf_dir.join("workflow.yaml")).unwrap();
        assert_eq!(wf.name, "process_document");
        assert_eq!(wf.path, wf_dir);
    }

    #[test]
    fn test_get_step() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        assert!(wf.get_step("read").is_some());
        assert!(wf.get_step("nonexistent").is_none());
    }

    #[test]
    fn test_step_ids() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        let ids = wf.step_ids();
        assert_eq!(ids, vec!["read", "summarize", "save"]);
    }

    #[test]
    fn test_required_variables() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        let required = wf.required_variables();
        assert_eq!(required, vec!["input_path"]);
    }

    #[test]
    fn test_has_tag() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        assert!(wf.has_tag("document"));
        assert!(wf.has_tag("pipeline"));
        assert!(!wf.has_tag("network"));
    }

    #[test]
    fn test_entry_steps() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        let entries = wf.entry_steps();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "read");
    }

    #[test]
    fn test_step_count() {
        let wf = Workflow::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(wf.step_count(), 3);
    }

    #[test]
    fn test_error_handling_strategy_display() {
        assert_eq!(ErrorHandlingStrategy::Abort.to_string(), "abort");
        assert_eq!(ErrorHandlingStrategy::Continue.to_string(), "continue");
        assert_eq!(ErrorHandlingStrategy::Retry.to_string(), "retry");
    }

    #[test]
    fn test_error_handling_strategy_serde() {
        let yaml = r#"
name: "test"
steps:
  - id: "s1"
    skill: "read_file"
error_handling:
  on_step_failure: "continue"
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        assert_eq!(
            wf.error_handling.on_step_failure,
            ErrorHandlingStrategy::Continue
        );
    }

    #[test]
    fn test_empty_steps() {
        let yaml = r#"
name: "empty_workflow"
steps: []
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        assert_eq!(wf.step_count(), 0);
        assert!(wf.entry_steps().is_empty());
    }
}
