//! Built-in system skill: `system-info` — report host system information.

/// The `skill.yaml` content for `system-info`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "system-info"
display_name: "System Info"
version: "1.0.0"
description: "Report OS, Python version, CPU count and total memory as JSON"
category: "system"

trigger_phrases:
  - "system info"
  - "host info"
  - "system information"
  - "系统信息"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  properties: {}

output_schema:
  type: "object"
  required: ["os", "python_version", "cpu_count"]
  properties:
    os:
      type: "string"
    os_version:
      type: "string"
    python_version:
      type: "string"
    processor:
      type: "string"
    cpu_count:
      type: "integer"
    total_memory:
      type: "string"
    cwd:
      type: "string"
    hostname:
      type: "string"

permissions:
  fs: []
  network: false
  shell: false

tags:
  - "system"
  - "diagnostics"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `system-info`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: system-info - report host system information."""

import json
import os
import platform
import sys


def main():
    mem = "unknown"
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    mem = line.split()[1] + " kB"
                    break
    except OSError:
        pass

    info = {
        "os": platform.system(),
        "os_version": platform.version(),
        "platform": platform.platform(),
        "python_version": platform.python_version(),
        "processor": platform.processor() or "unknown",
        "cpu_count": os.cpu_count(),
        "cwd": os.getcwd(),
        "total_memory": mem,
        "hostname": platform.node(),
    }
    print(json.dumps(info))


if __name__ == "__main__":
    main()
"#;

/// The basic example for `system-info`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: System Information

Report details about the host machine.

## Input

```json
{}
```

## Expected Output

```json
{"os": "Linux", "os_version": "1 SMP", "python_version": "3.11.1", "cpu_count": 8, "total_memory": "16384000 kB", "hostname": "caspian"}
```
"#;
