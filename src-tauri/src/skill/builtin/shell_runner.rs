//! Built-in system skill: `shell-runner` — execute a shell command.

/// The `skill.yaml` content for `shell-runner`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "shell-runner"
display_name: "Shell Runner"
version: "1.0.0"
description: "Execute a shell command and return its stdout, stderr and exit code"
category: "system"

trigger_phrases:
  - "run shell"
  - "execute command"
  - "run a command"
  - "运行命令"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 60
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["command"]
  properties:
    command:
      type: "string"
      description: "Command to execute"
    args:
      type: "array"
      items:
        type: "string"
      description: "Command arguments"
      default: []
    cwd:
      type: "string"
      description: "Working directory"
    timeout:
      type: "number"
      description: "Timeout in seconds (default: 60)"
      default: 60

output_schema:
  type: "object"
  required: ["stdout", "stderr", "exit_code", "duration_ms"]
  properties:
    stdout:
      type: "string"
    stderr:
      type: "string"
    exit_code:
      type: "integer"
    duration_ms:
      type: "integer"

permissions:
  fs:
    - read: ["{workspace}"]
      write: ["{workspace}"]
  network: false
  shell: true

tags:
  - "shell"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `shell-runner`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: shell-runner - execute a shell command and capture output."""

import json
import os
import subprocess
import sys
import time


def main():
    params = json.loads(sys.stdin.read())
    command = params.get("command", "")
    if not command:
        print(json.dumps({"error": "command is required"}))
        sys.exit(1)

    args = [str(a) for a in params.get("args", [])]
    cwd = os.path.expanduser(params.get("cwd", "")) or None
    timeout = params.get("timeout", 60)

    cmd = [command] + args
    start = time.time()
    try:
        r = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, cwd=cwd
        )
        print(json.dumps({
            "stdout": r.stdout,
            "stderr": r.stderr,
            "exit_code": r.returncode,
            "duration_ms": int((time.time() - start) * 1000),
        }))
    except subprocess.TimeoutExpired:
        print(json.dumps({
            "error": "command timed out",
            "duration_ms": int((time.time() - start) * 1000),
        }))
        sys.exit(1)
    except FileNotFoundError:
        print(json.dumps({"error": "command not found: " + command}))
        sys.exit(1)
    except OSError as e:
        print(json.dumps({"error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `shell-runner`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Run a Command

Run `echo` with an argument.

## Input

```json
{"command": "echo", "args": ["hello"]}
```

## Expected Output

```json
{"stdout": "hello\n", "stderr": "", "exit_code": 0, "duration_ms": 5}
```
"#;
