//! Built-in system skill: `skill-manager` — list installed skills on disk.

/// The `skill.yaml` content for `skill-manager`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "skill-manager"
display_name: "Skill Manager"
version: "1.0.0"
description: "List the skills currently installed in the Caspian skills directory"
category: "self"

trigger_phrases:
  - "list skills"
  - "show installed skills"
  - "what skills exist"
  - "列出技能"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  properties:
    skills_dir:
      type: "string"
      description: "Skills directory (default: ~/.caspian/skills)"
    base:
      type: "string"
      description: "Base directory (default: ~/.caspian)"
      default: "~/.caspian"

output_schema:
  type: "object"
  required: ["skills", "count"]
  properties:
    skills:
      type: "array"
      items:
        type: "object"
    count:
      type: "integer"

permissions:
  fs:
    - read: ["~/.caspian"]
  network: false
  shell: false

tags:
  - "self"
  - "skills"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `skill-manager`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: skill-manager - list installed skills by scanning the skills dir."""

import json
import os
import sys


def main():
    params = json.loads(sys.stdin.read())
    base = os.path.expanduser(params.get("base", "~/.caspian"))
    skills_dir = params.get("skills_dir", "") or os.path.join(base, "skills")
    skills_dir = os.path.expanduser(skills_dir)

    skills = []
    if os.path.isdir(skills_dir):
        for d in sorted(os.listdir(skills_dir)):
            yml = os.path.join(skills_dir, d, "skill.yaml")
            if os.path.isfile(yml):
                skills.append({"name": d, "manifest": yml})

    print(json.dumps({"skills": skills, "count": len(skills)}))


if __name__ == "__main__":
    main()
"#;

/// The basic example for `skill-manager`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: List Skills

Show all installed skills.

## Input

```json
{}
```

## Expected Output

```json
{"skills": [{"name": "read_file", "manifest": "/home/user/.caspian/skills/read_file/skill.yaml"}], "count": 1}
```
"#;
