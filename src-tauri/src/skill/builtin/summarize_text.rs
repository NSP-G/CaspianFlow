//! Built-in skill: `summarize_text` — extractive text summarization (stub for P23).

/// The `skill.yaml` content for `summarize_text`.
pub const SKILL_YAML: &str = r#"schema_version: "1.0"
name: "summarize_text"
display_name: "Summarize Text"
version: "1.0.0"
description: "Summarize text using extractive method (stub - will be upgraded to LLM in P23)"
category: "text"

trigger_phrases:
  - "summarize text"
  - "summarise"
  - "摘要"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"
  required: ["text"]
  properties:
    text:
      type: "string"
      description: "Text to summarize"
    max_length:
      type: "number"
      description: "Maximum summary length in characters (default: 200)"
      default: 200
    language:
      type: "string"
      description: "Text language for model selection (placeholder for P23)"
      default: "auto"

output_schema:
  type: "object"
  required: ["summary", "original_length", "summary_length", "method"]
  properties:
    summary:
      type: "string"
      description: "Generated summary"
    original_length:
      type: "number"
      description: "Original text length in characters"
    summary_length:
      type: "number"
      description: "Summary length in characters"
    method:
      type: "string"
      description: "Summarization method used"

permissions:
  fs: []
  network: false
  shell: false

tags:
  - "text"
  - "summary"
  - "builtin"

author: "Caspian Team"
license: "MIT"
"#;

/// The Python entry script for `summarize_text`.
///
/// Current implementation uses extractive summarization (sentence selection).
/// When P23 (model adapter) is ready, `extractive_summary()` will be replaced
/// with an LLM-based call. The `language` parameter is a placeholder for
/// P23 model/prompt selection and is not used in the stub.
pub const SCRIPT_PY: &str = r#"#!/usr/bin/env python3
"""Skill: summarize_text - extractive text summarization (stub for P23)."""

import json
import re
import sys


def extractive_summary(text, max_length=200, language=None):
    """Extractive summary: split by sentences, take first N until max_length.

    This is a stub implementation. When P23 (model adapter) is ready,
    this function will be replaced with an LLM-based summary.
    The `language` parameter is a placeholder for P23 model/prompt selection.
    """
    # Split by sentence-ending punctuation (supports both EN and CN)
    sentences = re.split(r'(?<=[.!?。！？])\s*', text.strip())
    sentences = [s.strip() for s in sentences if s.strip()]

    if not sentences:
        return ""

    summary_parts = []
    current_length = 0

    for sentence in sentences:
        if current_length + len(sentence) > max_length and summary_parts:
            break
        summary_parts.append(sentence)
        current_length += len(sentence)

    summary = " ".join(summary_parts)

    # Truncate if still over (single very long sentence)
    if len(summary) > max_length:
        summary = summary[:max_length]
        # Try to break at a word boundary
        last_space = summary.rfind(" ")
        if last_space > max_length * 0.5:
            summary = summary[:last_space]
        summary = summary + "..."

    return summary


def main():
    params = json.loads(sys.stdin.read())
    text = params.get("text", "")
    max_length = params.get("max_length", 200)
    language = params.get("language")  # placeholder for P23

    if not text:
        print(json.dumps({"error": "text is required"}))
        sys.exit(1)

    original_length = len(text)
    summary = extractive_summary(text, max_length, language)

    print(json.dumps({
        "summary": summary,
        "original_length": original_length,
        "summary_length": len(summary),
        "method": "extractive_stub"
    }))


if __name__ == "__main__":
    main()
"#;

/// The basic example for `summarize_text`.
pub const EXAMPLE_BASIC: &str = r#"# Basic Example: Summarize Text

Summarize a paragraph of text using extractive method.

## Input

```json
{"text": "This is the first sentence. This is the second sentence. This is the third sentence.", "max_length": 50}
```

## Expected Output

```json
{"summary": "This is the first sentence.", "original_length": 80, "summary_length": 25, "method": "extractive_stub"}
```

## Note

This is a stub implementation using extractive summarization. When P23 (model adapter)
is ready, this skill will be upgraded to use LLM-based summarization.
"#;
