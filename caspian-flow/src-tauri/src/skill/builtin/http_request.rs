//! Built-in skill: `http_request` — sends an HTTP request and returns the response.

/// The `skill.yaml` content for `http_request`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "http_request"
display_name: "HTTP Request"
version: "1.0.0"
description: "Send an HTTP request and return the response"
category: "network"

trigger_phrases:
  - "http request"
  - "fetch url"
  - "发送请求"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 60
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["url"]
  properties:
    url:
      type: "string"
      description: "Request URL"
    method:
      type: "string"
      description: "HTTP method (default: GET)"
      default: "GET"
    headers:
      type: "object"
      description: "Request headers"
    body:
      type: "string"
      description: "Request body"
    timeout:
      type: "number"
      description: "Timeout in seconds (default: 30)"
      default: 30

output_schema:
  type: "object"
  required: ["status_code", "headers", "body", "elapsed_ms"]
  properties:
    status_code:
      type: "number"
      description: "HTTP status code"
    headers:
      type: "object"
      description: "Response headers"
    body:
      type: "string"
      description: "Response body"
    elapsed_ms:
      type: "number"
      description: "Request duration in milliseconds"

permissions:
  fs: []
  network: true
  shell: false

tags:
  - "http"
  - "network"
  - "builtin"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `http_request`.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: http_request - sends an HTTP request and returns the response."""

import json
import sys
import time
import urllib.error
import urllib.request


def main():
    params = json.loads(sys.stdin.read())
    url = params.get("url", "")
    method = params.get("method", "GET").upper()
    headers = params.get("headers", {})
    body = params.get("body")
    timeout = params.get("timeout", 30)

    if not url:
        print(json.dumps({"error": "url is required"}))
        sys.exit(1)

    start = time.time()

    try:
        data = None
        if body is not None:
            if isinstance(body, str):
                data = body.encode("utf-8")
            else:
                data = json.dumps(body).encode("utf-8")

        req = urllib.request.Request(url, data=data, method=method)
        for key, value in headers.items():
            req.add_header(str(key), str(value))

        resp = urllib.request.urlopen(req, timeout=timeout)
        resp_body = resp.read().decode("utf-8", errors="replace")
        resp_headers = dict(resp.headers)
        status_code = resp.getcode()

        elapsed_ms = int((time.time() - start) * 1000)
        print(json.dumps({
            "status_code": status_code,
            "headers": resp_headers,
            "body": resp_body,
            "elapsed_ms": elapsed_ms
        }))

    except urllib.error.HTTPError as e:
        resp_body = ""
        try:
            resp_body = e.read().decode("utf-8", errors="replace")
        except Exception:
            pass
        elapsed_ms = int((time.time() - start) * 1000)
        print(json.dumps({
            "status_code": e.code,
            "headers": dict(e.headers) if e.headers else {},
            "body": resp_body,
            "elapsed_ms": elapsed_ms
        }))
    except urllib.error.URLError as e:
        elapsed_ms = int((time.time() - start) * 1000)
        print(json.dumps({
            "error": "network error: " + str(e.reason),
            "elapsed_ms": elapsed_ms
        }))
        sys.exit(1)
    except Exception as e:
        elapsed_ms = int((time.time() - start) * 1000)
        print(json.dumps({"error": str(e), "elapsed_ms": elapsed_ms}))
        sys.exit(1)


if __name__ == "__main__":
    main()
"#;

/// The basic example for `http_request`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: HTTP GET Request

Send a GET request to a URL.

## Input

```json
{"url": "https://httpbin.org/get", "method": "GET"}
```

## Expected Output

```json
{"status_code": 200, "headers": {"Content-Type": "application/json"}, "body": "{...}", "elapsed_ms": 150}
```
"#;
