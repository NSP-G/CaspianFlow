//! Built-in system skill: `json-parser` — validate, pretty-print or query JSON.

/// The `skill.yaml` content for `json-parser`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "json-parser"
display_name: "JSON Parser"
version: "1.0.0"
description: "Validate, pretty-print, or query a JSON document by dot path"
category: "data"

trigger_phrases:
  - "parse json"
  - "validate json"
  - "query json"
  - "解析 JSON"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["json"]
  properties:
    json:
      type: "string"
      description: "Raw JSON string"
    action:
      type: "string"
      description: "validate | pretty | query (default: validate)"
      default: "validate"
    path:
      type: "string"
      description: "Dot path for query action, e.g. user.address.city"

output_schema:
  type: "object"
  required: ["valid"]
  properties:
    valid:
      type: "boolean"
    type:
      type: "string"
    pretty:
      type: "string"
    value:
      type: "object"

permissions:
  fs: []
  network: false
  shell: false

tags:
  - "data"
  - "json"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `json-parser`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: json-parser - validate, pretty-print or query JSON."""

import json
import sys


def main():
    params = json.loads(sys.stdin.read())
    text = params.get("json", "")
    action = params.get("action", "validate")

    if not text:
        print(json.dumps({"error": "json is required"}))
        sys.exit(1)

    try:
        data = json.loads(text)
    except json.JSONDecodeError as e:
        print(json.dumps({"valid": False, "error": str(e)}))
        sys.exit(1)

    if action == "validate":
        print(json.dumps({"valid": True, "type": type(data).__name__}))
    elif action == "pretty":
        print(json.dumps({
            "valid": True,
            "pretty": json.dumps(data, indent=2, ensure_ascii=False),
        }))
    elif action == "query":
        path = params.get("path", "")
        cur = data
        try:
            for key in path.split("."):
                if key == "":
                    continue
                if isinstance(cur, list):
                    cur = cur[int(key)]
                else:
                    cur = cur[key]
            print(json.dumps({"valid": True, "value": cur}))
        except (KeyError, IndexError, ValueError) as e:
            print(json.dumps({"valid": True, "error": "path not found: " + str(e)}))
            sys.exit(1)
    else:
        print(json.dumps({"error": "unknown action: " + action}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `json-parser`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Pretty-Print JSON

Pretty-print a compact JSON document.

## Input

```json
{"json": "{\"a\":1,\"b\":[2,3]}", "action": "pretty"}
```

## Expected Output

```json
{"valid": true, "pretty": "{\n  \"a\": 1,\n  \"b\": [\n    2,\n    3\n  ]\n}"}
```
"#;
