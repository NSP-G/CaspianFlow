//! Built-in system skill: `memory-manager` — read/update a persistent memory file.

/// The `skill.yaml` content for `memory-manager`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "memory-manager"
display_name: "Memory Manager"
version: "1.0.0"
description: "Read or update a persistent markdown memory file (MEMORY.md)"
category: "self"

trigger_phrases:
  - "read memory"
  - "update memory"
  - "remember that"
  - "记忆管理"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["action"]
  properties:
    action:
      type: "string"
      description: "get | set | append (default: get)"
      default: "get"
    text:
      type: "string"
      description: "Text to write for set/append"
    base:
      type: "string"
      description: "Base directory (default: ~/.caspian)"
      default: "~/.caspian"

output_schema:
  type: "object"
  required: ["action"]
  properties:
    content:
      type: "string"
    path:
      type: "string"
    action:
      type: "string"
    updated:
      type: "boolean"

permissions:
  fs:
    - read: ["{workspace}", "~/.caspian"]
      write: ["{workspace}", "~/.caspian"]
  network: false
  shell: false

tags:
  - "self"
  - "memory"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `memory-manager`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: memory-manager - read or update a persistent MEMORY.md file."""

import json
import os
import sys


def main():
    params = json.loads(sys.stdin.read())
    action = params.get("action", "get")
    base = os.path.expanduser(params.get("base", "~/.caspian"))
    path = os.path.join(base, "MEMORY.md")
    os.makedirs(base, exist_ok=True)

    if action == "get":
        content = ""
        if os.path.exists(path):
            with open(path, "r", encoding="utf-8") as f:
                content = f.read()
        print(json.dumps({"content": content, "action": "get"}))
    elif action in ("set", "append"):
        text = params.get("text", "")
        mode = "w" if action == "set" else "a"
        with open(path, mode, encoding="utf-8") as f:
            f.write(text + ("\n" if not text.endswith("\n") else ""))
        print(json.dumps({"path": path, "action": action, "updated": True}))
    else:
        print(json.dumps({"error": "unknown action: " + action}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `memory-manager`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Update Memory

Append a fact to the persistent memory file.

## Input

```json
{"action": "append", "text": "- User prefers concise summaries."}
```

## Expected Output

```json
{"path": "/home/user/.caspian/MEMORY.md", "action": "append", "updated": true}
```
"#;
