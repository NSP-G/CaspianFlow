//! Built-in system skill: `web-fetcher` — fetch a URL over HTTP(S).

/// The `skill.yaml` content for `web-fetcher`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "web-fetcher"
display_name: "Web Fetcher"
version: "1.0.0"
description: "Fetch a URL over HTTP(S) and return status, headers and body"
category: "network"

trigger_phrases:
  - "fetch url"
  - "download webpage"
  - "http get"
  - "抓取网页"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["url"]
  properties:
    url:
      type: "string"
      description: "The URL to fetch"
    method:
      type: "string"
      description: "HTTP method (default: GET)"
      default: "GET"
    headers:
      type: "object"
      description: "Extra request headers"
    body:
      type: "string"
      description: "Request body (for POST/PUT)"

output_schema:
  type: "object"
  required: ["status", "body", "size"]
  properties:
    status:
      type: "integer"
    headers:
      type: "object"
    body:
      type: "string"
    size:
      type: "integer"

permissions:
  fs: []
  network: true
  shell: false

tags:
  - "network"
  - "http"
  - "system"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `web-fetcher`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: web-fetcher - fetch a URL using only the standard library."""

import json
import sys
import urllib.error
import urllib.request


def main():
    params = json.loads(sys.stdin.read())
    url = params.get("url", "")
    if not url:
        print(json.dumps({"error": "url is required"}))
        sys.exit(1)

    method = str(params.get("method", "GET")).upper()
    headers = params.get("headers", {})
    body = params.get("body", "")

    req = urllib.request.Request(
        url, data=body.encode("utf-8") if body else None, method=method
    )
    for k, v in headers.items():
        req.add_header(k, v)

    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = resp.read()
            charset = resp.headers.get_content_charset() or "utf-8"
            text = data.decode(charset, errors="replace")
            print(json.dumps({
                "status": resp.status,
                "headers": dict(resp.headers),
                "body": text,
                "size": len(data),
            }))
    except urllib.error.HTTPError as e:
        print(json.dumps({"error": "http error " + str(e.code), "status": e.code}))
        sys.exit(1)
    except urllib.error.URLError as e:
        print(json.dumps({"error": "url error: " + str(e.reason)}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `web-fetcher`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Fetch a Web Page

Retrieve the contents of a URL.

## Input

```json
{"url": "https://example.com", "method": "GET"}
```

## Expected Output

```json
{"status": 200, "headers": {"Content-Type": "text/html"}, "body": "<html>...</html>", "size": 1256}
```
"#;
