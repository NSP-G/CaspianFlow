//! Built-in system skill: `file-writer` — writes content to a local file.

/// The `skill.yaml` content for `file-writer`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "file-writer"
display_name: "File Writer"
version: "1.0.0"
description: "Write or append text content to a local file"
category: "file"

trigger_phrases:
  - "write file"
  - "save file"
  - "append to file"
  - "写入文件"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["path", "content"]
  properties:
    path:
      type: "string"
      description: "Path of the file to write"
    content:
      type: "string"
      description: "Content to write"
    append:
      type: "boolean"
      description: "Append instead of overwrite (default: false)"
      default: false

output_schema:
  type: "object"
  required: ["path", "bytes_written", "appended"]
  properties:
    path:
      type: "string"
    bytes_written:
      type: "integer"
    appended:
      type: "boolean"
    size:
      type: "integer"

permissions:
  fs:
    - read: ["{workspace}", "~/.caspian"]
      write: ["{workspace}", "~/.caspian"]
  network: false
  shell: false

tags:
  - "file"
  - "write"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `file-writer`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: file-writer - writes content to a local file."""

import json
import os
import sys


def main():
    params = json.loads(sys.stdin.read())
    path = params.get("path", "")
    content = params.get("content", "")
    append = bool(params.get("append", False))

    if not path:
        print(json.dumps({"error": "path is required"}))
        sys.exit(1)

    path = os.path.expanduser(path)
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)

    mode = "a" if append else "w"
    try:
        with open(path, mode, encoding="utf-8") as f:
            f.write(content)
        print(json.dumps({
            "path": path,
            "bytes_written": len(content.encode("utf-8")),
            "appended": append,
            "size": os.path.getsize(path),
        }))
    except OSError as e:
        print(json.dumps({"error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `file-writer`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Write a File

Write text to a file (overwrites existing content).

## Input

```json
{"path": "/tmp/note.txt", "content": "Hello, CaspianFlow!"}
```

## Expected Output

```json
{"path": "/tmp/note.txt", "bytes_written": 19, "appended": false, "size": 19}
```
"#;
