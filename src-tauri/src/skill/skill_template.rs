//! Default skill template — generates a new skill directory scaffold.

use std::path::Path;

use crate::types::AppResult;

/// The default `skill.yaml` template content.
pub const DEFAULT_SKILL_YAML: &str = r#"schema_version: "1.0"
name: "{{SKILL_NAME}}"
display_name: "{{DISPLAY_NAME}}"
version: "1.0.0"
description: "TODO: Describe what this skill does"
category: "utility"

trigger_phrases:
  - "TODO: Add trigger phrase"
  - "TODO: Add another trigger phrase"

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
      description: "Input parameter"

output_schema:
  type: "object"
  required: ["result"]
  properties:
    result:
      type: "string"
      description: "Output result"

permissions:
  fs: []
  network: false
  shell: false

tags:
  - "utility"

author: ""
license: "MIT"
"#;

/// Default Python entry script template.
const DEFAULT_SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill entry point — receives JSON params on stdin, outputs JSON on stdout."""

import json
import sys


def main():
    params = json.loads(sys.stdin.read())
    input_value = params.get("input", "")
    result = {"result": f"Processed: {input_value}"}
    print(json.dumps(result))


if __name__ == "__main__":
    main()
"#;

/// Default basic example template.
const DEFAULT_EXAMPLE: &str = r#"# Basic Example

User: TODO: Add example user input
Output: {"input": "example_value"}
"#;

/// Default README template.
const DEFAULT_README: &str = r#"# {{DISPLAY_NAME}}

TODO: Add skill documentation here.

## Usage

Describe how to use this skill.

## Parameters

- `input`: Input parameter

## Examples

See the `examples/` directory for few-shot examples.
"#;

/// Convert a skill name (e.g. `read_file`) to a display name (e.g. `Read File`).
fn to_display_name(name: &str) -> String {
    name.split(['_', '-'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Create a new skill directory with template files.
///
/// Creates the following structure:
/// ```text
/// <dir>/
/// ├── skill.yaml
/// ├── README.md
/// ├── script.py
/// ├── examples/
/// │   └── 01_basic.md
/// └── assets/
/// ```
pub fn create_skill_template(dir: &Path, name: &str) -> AppResult<()> {
    let display_name = to_display_name(name);

    // Create directory structure
    std::fs::create_dir_all(dir)?;
    std::fs::create_dir_all(dir.join("examples"))?;
    std::fs::create_dir_all(dir.join("assets"))?;

    // Write skill.yaml with name substituted
    let yaml = DEFAULT_SKILL_YAML
        .replace("{{SKILL_NAME}}", name)
        .replace("{{DISPLAY_NAME}}", &display_name);
    std::fs::write(dir.join("skill.yaml"), yaml)?;

    // Write README.md
    let readme = DEFAULT_README.replace("{{DISPLAY_NAME}}", &display_name);
    std::fs::write(dir.join("README.md"), readme)?;

    // Write script.py
    std::fs::write(dir.join("script.py"), DEFAULT_SCRIPT_PY)?;

    // Write basic example
    std::fs::write(dir.join("examples").join("01_basic.md"), DEFAULT_EXAMPLE)?;

    tracing::info!(
        dir = %dir.display(),
        name = %name,
        "skill template created"
    );

    Ok(())
}

/// Get the template as a YAML string without writing to disk.
pub fn template_yaml(name: &str) -> String {
    let display_name = to_display_name(name);
    DEFAULT_SKILL_YAML
        .replace("{{SKILL_NAME}}", name)
        .replace("{{DISPLAY_NAME}}", &display_name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::Skill;

    #[test]
    fn test_to_display_name() {
        assert_eq!(to_display_name("read_file"), "Read File");
        assert_eq!(to_display_name("read-file"), "Read File");
        assert_eq!(to_display_name("simple"), "Simple");
        assert_eq!(to_display_name("multi_word_name"), "Multi Word Name");
    }

    #[test]
    fn test_template_yaml_substitution() {
        let yaml = template_yaml("my_skill");
        assert!(yaml.contains("name: \"my_skill\""));
        assert!(yaml.contains("display_name: \"My Skill\""));
    }

    #[test]
    fn test_create_skill_template() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("test_skill");

        create_skill_template(&skill_dir, "test_skill").unwrap();

        // Check files exist
        assert!(skill_dir.join("skill.yaml").exists());
        assert!(skill_dir.join("README.md").exists());
        assert!(skill_dir.join("script.py").exists());
        assert!(skill_dir.join("examples").exists());
        assert!(skill_dir.join("examples").join("01_basic.md").exists());
        assert!(skill_dir.join("assets").exists());
    }

    #[test]
    fn test_template_skill_yaml_is_valid() {
        let yaml = template_yaml("my_skill");

        // The template YAML should parse successfully into a Skill
        let skill = Skill::from_yaml(&yaml).unwrap();
        assert_eq!(skill.name, "my_skill");
        assert_eq!(skill.display_name, "My Skill");
        assert_eq!(skill.version, "1.0.0");
        assert_eq!(skill.category, "utility");
        assert_eq!(skill.runtime.entry, "script.py");
    }

    #[test]
    fn test_create_template_in_existing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("existing_skill");
        std::fs::create_dir_all(&skill_dir).unwrap();

        // Should not fail even if directory exists
        create_skill_template(&skill_dir, "existing_skill").unwrap();
        assert!(skill_dir.join("skill.yaml").exists());
    }
}
