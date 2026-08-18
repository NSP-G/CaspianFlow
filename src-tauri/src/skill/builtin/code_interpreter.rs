//! Built-in system skill: `code-interpreter` — run a Python snippet.

/// The `skill.yaml` content for `code-interpreter`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "code-interpreter"
display_name: "Code Interpreter"
version: "1.0.0"
description: "Execute a Python code snippet and capture its standard output"
category: "system"

trigger_phrases:
  - "run python"
  - "execute code"
  - "code interpreter"
  - "运行代码"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["code"]
  properties:
    code:
      type: "string"
      description: "Python source code to execute"

output_schema:
  type: "object"
  required: ["stdout", "error"]
  properties:
    stdout:
      type: "string"
    error:
      type: "string"

permissions:
  fs: []
  network: false
  shell: false

tags:
  - "system"
  - "code"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `code-interpreter`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: code-interpreter - run a Python snippet and capture stdout."""

import contextlib
import io
import json
import sys


def main():
    params = json.loads(sys.stdin.read())
    code = params.get("code", "")
    if not code:
        print(json.dumps({"error": "code is required"}))
        sys.exit(1)

    buf = io.StringIO()
    local_ns = {}
    try:
        with contextlib.redirect_stdout(buf):
            exec(compile(code, "<skill>", "exec"),
                 {"__name__": "__main__"}, local_ns)
        print(json.dumps({"stdout": buf.getvalue(), "error": None}))
    except Exception as e:  # noqa: BLE001 - report any failure to caller
        print(json.dumps({"stdout": buf.getvalue(), "error": repr(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `code-interpreter`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Run Code

Execute a short Python snippet.

## Input

```json
{"code": "print(sum(range(1, 11)))"}
```

## Expected Output

```json
{"stdout": "55\n", "error": null}
```
"#;
