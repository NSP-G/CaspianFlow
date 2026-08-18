//! Built-in system skill: `workflow-runner` — list available workflow definitions.

/// The `skill.yaml` content for `workflow-runner`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "workflow-runner"
display_name: "Workflow Runner"
version: "1.0.0"
description: "List workflow definitions available in the workflows directory"
category: "self"

trigger_phrases:
  - "list workflows"
  - "show workflows"
  - "available workflows"
  - "列出工作流"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  properties:
    workflows_dir:
      type: "string"
      description: "Workflows directory (default: ~/.caspian/workflows)"
    base:
      type: "string"
      description: "Base directory (default: ~/.caspian)"
      default: "~/.caspian"

output_schema:
  type: "object"
  required: ["workflows", "count", "directory"]
  properties:
    workflows:
      type: "array"
      items:
        type: "string"
    count:
      type: "integer"
    directory:
      type: "string"

permissions:
  fs:
    - read: ["~/.caspian"]
  network: false
  shell: false

tags:
  - "self"
  - "workflow"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `workflow-runner`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: workflow-runner - list available workflow definition files."""

import json
import os
import sys


def main():
    params = json.loads(sys.stdin.read())
    base = os.path.expanduser(params.get("base", "~/.caspian"))
    wf_dir = params.get("workflows_dir", "") or os.path.join(base, "workflows")
    wf_dir = os.path.expanduser(wf_dir)

    workflows = []
    if os.path.isdir(wf_dir):
        for f in sorted(os.listdir(wf_dir)):
            if f.endswith(".yaml") or f.endswith(".yml"):
                workflows.append(f)

    print(json.dumps({
        "workflows": workflows,
        "count": len(workflows),
        "directory": wf_dir,
    }))


if __name__ == "__main__":
    main()
"#;

/// The basic example for `workflow-runner`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: List Workflows

Show workflow files in the workflows directory.

## Input

```json
{}
```

## Expected Output

```json
{"workflows": ["deploy.yaml", "backup.yml"], "count": 2, "directory": "/home/user/.caspian/workflows"}
```
"#;
