//! Built-in system skill: `note-taker` — append timestamped notes to a file.

/// The `skill.yaml` content for `note-taker`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "note-taker"
display_name: "Note Taker"
version: "1.0.0"
description: "Append a timestamped note to notes.md in the workspace base directory"
category: "self"

trigger_phrases:
  - "take a note"
  - "save note"
  - "remember this"
  - "记笔记"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["body"]
  properties:
    title:
      type: "string"
      description: "Note title (default: note)"
      default: "note"
    body:
      type: "string"
      description: "Note body text"
    base:
      type: "string"
      description: "Base directory (default: ~/.caspian)"
      default: "~/.caspian"

output_schema:
  type: "object"
  required: ["path", "appended", "timestamp"]
  properties:
    path:
      type: "string"
    appended:
      type: "boolean"
    timestamp:
      type: "string"

permissions:
  fs:
    - write: ["{workspace}", "~/.caspian"]
  network: false
  shell: false

tags:
  - "self"
  - "notes"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `note-taker`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: note-taker - append a timestamped note to notes.md."""

import json
import os
import sys
from datetime import datetime


def main():
    params = json.loads(sys.stdin.read())
    title = params.get("title", "note")
    body = params.get("body", "")
    base = os.path.expanduser(params.get("base", "~/.caspian"))

    if not body:
        print(json.dumps({"error": "body is required"}))
        sys.exit(1)

    os.makedirs(base, exist_ok=True)
    path = os.path.join(base, "notes.md")
    ts = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    entry = "\n## " + title + " (" + ts + ")\n\n" + body + "\n"

    with open(path, "a", encoding="utf-8") as f:
        f.write(entry)

    print(json.dumps({"path": path, "appended": True, "timestamp": ts}))


if __name__ == "__main__":
    main()
"#;

/// The basic example for `note-taker`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Take a Note

Append a timestamped note.

## Input

```json
{"title": "Idea", "body": "Build a CLI wrapper for the API."}
```

## Expected Output

```json
{"path": "/home/user/.caspian/notes.md", "appended": true, "timestamp": "2026-08-16 10:30:00"}
```
"#;
