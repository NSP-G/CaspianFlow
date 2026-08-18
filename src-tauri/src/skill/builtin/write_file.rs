//! Built-in skill: `write_file` — writes content to a local file.

/// The `skill.yaml` content for `write_file`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "write_file"
display_name: "Write File"
version: "1.0.0"
description: "Write content to a local file"
category: "file-system"

trigger_phrases:
  - "write file"
  - "save file"
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
      description: "File path to write"
    content:
      type: "string"
      description: "Content to write"
    append:
      type: "boolean"
      description: "Append mode (default: false)"
      default: false

output_schema:
  type: "object"
  required: ["path", "bytes_written", "appended"]
  properties:
    path:
      type: "string"
      description: "Absolute file path written"
    bytes_written:
      type: "number"
      description: "Number of bytes written"
    appended:
      type: "boolean"
      description: "Whether append mode was used"

permissions:
  fs:
    - read: ["{workspace}"]
      write: ["{workspace}"]
  network: false
  shell: false

tags:
  - "file"
  - "write"
  - "builtin"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `write_file`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: write_file - writes content to a local file."""

import json
import os
import sys


def main():
    params = json.loads(sys.stdin.read())
    path = params.get("path", "")
    content = params.get("content", "")
    append = params.get("append", False)

    if not path:
        print(json.dumps({"error": "path is required"}))
        sys.exit(1)

    # Expand ~ to home directory
    path = os.path.expanduser(path)

    # Create parent directories if needed
    parent = os.path.dirname(path)
    if parent and not os.path.exists(parent):
        os.makedirs(parent, exist_ok=True)

    try:
        mode = "a" if append else "w"
        with open(path, mode, encoding="utf-8") as f:
            f.write(content)

        bytes_written = len(content.encode("utf-8"))
        print(json.dumps({
            "path": path,
            "bytes_written": bytes_written,
            "appended": append
        }))
    except OSError as e:
        print(json.dumps({"error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `write_file`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Write a File

Write content to a file on the local filesystem.

## Input

```json
{"path": "/tmp/output.txt", "content": "Hello, World!", "append": false}
```

## Expected Output

```json
{"path": "/tmp/output.txt", "bytes_written": 13, "appended": false}
```
"#;
