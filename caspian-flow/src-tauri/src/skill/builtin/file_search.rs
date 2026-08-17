//! Built-in system skill: `file-search` — recursive text search across files.

/// The `skill.yaml` content for `file-search`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "file-search"
display_name: "File Search"
version: "1.0.0"
description: "Recursively search files in a directory for a regex pattern (grep)"
category: "file"

trigger_phrases:
  - "search files"
  - "grep"
  - "find in files"
  - "搜索文件"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 60
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["directory", "pattern"]
  properties:
    directory:
      type: "string"
      description: "Directory to search recursively"
    pattern:
      type: "string"
      description: "Regular expression to match"
    max_depth:
      type: "integer"
      description: "Maximum directory depth (default: 5)"
      default: 5
    extensions:
      type: "array"
      items:
        type: "string"
      description: "Only search files with these extensions"
    case_sensitive:
      type: "boolean"
      description: "Case-sensitive matching (default: false)"
      default: false

output_schema:
  type: "object"
  required: ["pattern", "matches", "count"]
  properties:
    pattern:
      type: "string"
    matches:
      type: "array"
      items:
        type: "object"
    count:
      type: "integer"

permissions:
  fs:
    - read: ["{workspace}", "~/.caspian"]
  network: false
  shell: false

tags:
  - "file"
  - "search"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `file-search`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: file-search - recursive grep across a directory tree."""

import json
import os
import re
import sys


def main():
    params = json.loads(sys.stdin.read())
    directory = params.get("directory", "")
    pattern = params.get("pattern", "")

    if not directory:
        print(json.dumps({"error": "directory is required"}))
        sys.exit(1)
    if not pattern:
        print(json.dumps({"error": "pattern is required"}))
        sys.exit(1)

    directory = os.path.expanduser(directory)
    if not os.path.isdir(directory):
        print(json.dumps({"error": "not a directory: " + directory}))
        sys.exit(1)

    max_depth = int(params.get("max_depth", 5))
    exts = params.get("extensions", [])
    flags = 0 if params.get("case_sensitive", False) else re.IGNORECASE

    try:
        regex = re.compile(pattern, flags)
    except re.error as e:
        print(json.dumps({"error": "invalid regex: " + str(e)}))
        sys.exit(1)

    matches = []
    for root, dirs, files in os.walk(directory):
        depth = root[len(directory):].count(os.sep)
        if depth > max_depth:
            continue
        for fname in files:
            if exts and not any(fname.endswith(e) for e in exts):
                continue
            fpath = os.path.join(root, fname)
            try:
                with open(fpath, "r", encoding="utf-8", errors="ignore") as f:
                    for i, line in enumerate(f, 1):
                        if regex.search(line):
                            matches.append({
                                "file": fpath,
                                "line": i,
                                "text": line.rstrip("\n"),
                            })
            except OSError:
                continue
            if len(matches) >= 200:
                break
        if len(matches) >= 200:
            break

    print(json.dumps({"pattern": pattern, "matches": matches, "count": len(matches)}))


if __name__ == "__main__":
    main()
"#;

/// The basic example for `file-search`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Search for a Pattern

Find every line containing "error" under a project directory.

## Input

```json
{"directory": "/path/to/project", "pattern": "error", "extensions": [".py", ".rs"]}
```

## Expected Output

```json
{"pattern": "error", "matches": [{"file": "/path/to/project/main.py", "line": 12, "text": "log.error('failed')"}], "count": 1}
```
"#;
