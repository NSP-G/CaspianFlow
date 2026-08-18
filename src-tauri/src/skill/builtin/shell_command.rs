//! Built-in skill: `shell_command` — executes a shell command and returns output.

/// The `skill.yaml` content for `shell_command`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "shell_command"
display_name: "Shell Command"
version: "1.0.0"
description: "Execute a shell command and return stdout, stderr, and exit code"
category: "system"

trigger_phrases:
  - "run command"
  - "execute command"
  - "执行命令"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
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
      description: "Timeout in seconds (default: 30)"
      default: 30

output_schema:
  type: "object"
  required: ["stdout", "stderr", "exit_code", "duration_ms"]
  properties:
    stdout:
      type: "string"
      description: "Standard output"
    stderr:
      type: "string"
      description: "Standard error"
    exit_code:
      type: "number"
      description: "Process exit code"
    duration_ms:
      type: "number"
      description: "Execution duration in milliseconds"

permissions:
  fs:
    - read: ["{workspace}"]
      write: ["{workspace}"]
  network: false
  shell: true

tags:
  - "shell"
  - "system"
  - "builtin"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `shell_command`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: shell_command - executes a shell command and returns output."""

import json
import os
import subprocess
import sys
import time


def main():
    params = json.loads(sys.stdin.read())
    command = params.get("command", "")
    args = params.get("args", [])
    cwd = params.get("cwd", "")
    timeout = params.get("timeout", 30)

    if not command:
        print(json.dumps({"error": "command is required"}))
        sys.exit(1)

    if cwd:
        cwd = os.path.expanduser(cwd)

    # Build command list — no shell=True to prevent injection
    cmd_list = [command] + [str(a) for a in args]
    start = time.time()

    try:
        result = subprocess.run(
            cmd_list,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=cwd if cwd else None,
        )
        elapsed_ms = int((time.time() - start) * 1000)
        print(json.dumps({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.returncode,
            "duration_ms": elapsed_ms
        }))
    except subprocess.TimeoutExpired:
        elapsed_ms = int((time.time() - start) * 1000)
        print(json.dumps({
            "error": "command timed out after " + str(timeout) + "s",
            "duration_ms": elapsed_ms
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

/// The basic example for `shell_command`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Run a Command

Execute the `echo` command with an argument.

## Input

```json
{"command": "echo", "args": ["Hello, World!"]}
```

## Expected Output

```json
{"stdout": "Hello, World!\n", "stderr": "", "exit_code": 0, "duration_ms": 5}
```
"#;
