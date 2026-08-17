//! Built-in skill: `read_file` — reads a local text file and returns its content.

/// The `skill.yaml` content for `read_file`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "read_file"
display_name: "Read File"
version: "1.0.0"
description: "Read the contents of a local text file"
category: "file-system"

trigger_phrases:
  - "read file"
  - "open file"
  - "读取文件"

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
      description: "File path to read"
    encoding:
      type: "string"
      description: "File encoding (default: utf-8)"
      default: "utf-8"

output_schema:
  type: "object"
  required: ["content", "size", "encoding"]
  properties:
    content:
      type: "string"
      description: "File content"
    size:
      type: "number"
      description: "Content size in bytes"
    encoding:
      type: "string"
      description: "Encoding used"

permissions:
  fs:
    - read: ["{workspace}", "~/.caspian"]
  network: false
  shell: false

tags:
  - "file"
  - "read"
  - "builtin"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `read_file`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: read_file - reads a local text file and returns its content."""

import json
import os
import sys


def main():
    params = json.loads(sys.stdin.read())
    path = params.get("path", "")
    encoding = params.get("encoding", "utf-8")

    if not path:
        print(json.dumps({"error": "path is required"}))
        sys.exit(1)

    # Expand ~ to home directory
    path = os.path.expanduser(path)

    if not os.path.exists(path):
        print(json.dumps({"error": "file not found: " + path}))
        sys.exit(1)

    if not os.path.isfile(path):
        print(json.dumps({"error": "not a file: " + path}))
        sys.exit(1)

    try:
        with open(path, "r", encoding=encoding) as f:
            content = f.read()
        size = len(content.encode(encoding))
        print(json.dumps({"content": content, "size": size, "encoding": encoding}))
    except UnicodeDecodeError as e:
        print(json.dumps({"error": "encoding error: " + str(e)}))
        sys.exit(1)
    except OSError as e:
        print(json.dumps({"error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `read_file`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Read a File

Read a text file from the local filesystem.

## Input

```json
{"path": "/tmp/hello.txt", "encoding": "utf-8"}
```

## Expected Output

```json
{"content": "Hello, World!", "size": 13, "encoding": "utf-8"}
```
"#;
