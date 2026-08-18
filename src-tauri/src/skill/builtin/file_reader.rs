//! Built-in system skill: `file-reader` — reads a local text file.

/// The `skill.yaml` content for `file-reader`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "file-reader"
display_name: "File Reader"
version: "1.0.0"
description: "Read a local text file and return its content, line count and size"
category: "file"

trigger_phrases:
  - "read file"
  - "open file"
  - "read a text file"
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
      description: "Path of the file to read"
    encoding:
      type: "string"
      description: "Text encoding (default: utf-8)"
      default: "utf-8"

output_schema:
  type: "object"
  required: ["content", "line_count", "size", "encoding"]
  properties:
    content:
      type: "string"
      description: "File content"
    line_count:
      type: "integer"
      description: "Number of lines"
    size:
      type: "integer"
      description: "Byte size of the content"
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
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `file-reader`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: file-reader - reads a local text file."""

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

    path = os.path.expanduser(path)

    if not os.path.isfile(path):
        print(json.dumps({"error": "file not found: " + path}))
        sys.exit(1)

    try:
        with open(path, "r", encoding=encoding) as f:
            content = f.read()
        print(json.dumps({
            "content": content,
            "line_count": content.count("\n") + 1,
            "size": len(content.encode(encoding)),
            "encoding": encoding,
        }))
    except (UnicodeDecodeError, OSError) as e:
        print(json.dumps({"error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `file-reader`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Read a File

Read a text file from disk and inspect its contents.

## Input

```json
{"path": "/tmp/hello.txt", "encoding": "utf-8"}
```

## Expected Output

```json
{"content": "Hello, World!\n", "line_count": 2, "size": 14, "encoding": "utf-8"}
```
"#;
