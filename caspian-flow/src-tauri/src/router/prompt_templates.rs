//! Prompt templates for slot filling.
//!
//! This module handles the assembly of:
//! - Extraction prompts (skill description + schema + few-shot + user input)
//! - Correction prompts (previous output + error list + retry instruction)
//!
//! The templates are designed to be language-agnostic (work with both
//! Chinese and English input).

use serde_json::Value;

use crate::skill::examples::SkillExample;

/// Build the parameter extraction prompt.
///
/// The prompt structure:
/// ```text
/// 你是一个参数提取助手。请从用户输入中提取以下 Skill 所需的参数，以 JSON 格式输出。
///
/// Skill: {display_name}
/// 描述: {description}
///
/// 输入参数 Schema:
/// {pretty_schema}
///
/// 示例 1:
/// 用户输入: "..."
/// 输出: {...}
///
/// 示例 2:
/// 用户输入: "..."
/// 输出: {...}
///
/// 用户输入: "{user_input}"
/// 输出:
/// ```
pub fn build_extraction_prompt(
    display_name: &str,
    description: &str,
    input_schema: &Value,
    examples: &[SkillExample],
    user_input: &str,
) -> String {
    let schema_str = format_schema(input_schema);
    let examples_str = format_examples(examples, 3);

    format!(
        r#"你是一个参数提取助手。请从用户输入中提取以下 Skill 所需的参数，以 JSON 格式输出。

Skill: {display_name}
描述: {description}

输入参数 Schema:
{schema_str}
{examples_str}
用户输入: "{user_input}"
输出:"#
    )
}

/// Build the correction prompt for a retry attempt.
///
/// Includes the previous output and specific errors, asking the LLM
/// to fix and re-output valid JSON.
pub fn build_correction_prompt(
    display_name: &str,
    description: &str,
    input_schema: &Value,
    examples: &[SkillExample],
    user_input: &str,
    previous_output: &str,
    errors: &[String],
) -> String {
    let schema_str = format_schema(input_schema);
    let examples_str = format_examples(examples, 2); // Fewer examples in retry to save tokens
    let error_list = errors
        .iter()
        .enumerate()
        .map(|(i, e)| format!("  {}. {}", i + 1, e))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"你是一个参数提取助手。你上次的输出存在问题，请修正后重新输出。

Skill: {display_name}
描述: {description}

输入参数 Schema:
{schema_str}
{examples_str}
用户输入: "{user_input}"

你上次的输出:
{previous_output}

你输出的 JSON 有以下问题：
{error_list}

请修正以上问题，重新输出完整的 JSON 对象。只输出 JSON，不要包含其他文本。
输出:"#
    )
}

/// Pretty-print a JSON Schema for inclusion in the prompt.
///
/// Uses 2-space indentation. If the schema is empty, returns "{}".
fn format_schema(schema: &Value) -> String {
    if schema.is_null() {
        return "{}".to_string();
    }
    serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string())
}

/// Format few-shot examples for inclusion in the prompt.
///
/// Takes up to `max` examples. Each example is expected to be in the
/// format:
/// ```markdown
/// 用户输入: "..."
/// 输出: {...}
/// ```
///
/// If the example content doesn't follow this format, it's included as-is.
fn format_examples(examples: &[SkillExample], max: usize) -> String {
    if examples.is_empty() {
        return String::new();
    }

    let count = examples.len().min(max);
    let mut parts = Vec::new();

    for (i, example) in examples.iter().take(count).enumerate() {
        let content = example.content.trim();
        if content.is_empty() {
            continue;
        }
        parts.push(format!("示例 {}:\n{}", i + 1, content));
    }

    if parts.is_empty() {
        return String::new();
    }

    format!("\n{}\n", parts.join("\n\n"))
}

/// Generate a user-facing question for a missing required field.
///
/// This is used when the slot filler returns `NeedsUserInput` and the
/// IPC layer needs to present the question to the user.
pub fn format_missing_field_question(
    field_name: &str,
    description: &str,
    param_type: &str,
) -> String {
    if description.is_empty() {
        format!("请提供「{field_name}」的值（类型: {param_type}）")
    } else {
        format!("请提供「{field_name}」的值 — {description}（类型: {param_type}）")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_extraction_prompt_basic() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string", "description": "文件路径" }
            }
        });

        let prompt = build_extraction_prompt(
            "读取文件",
            "读取本地文本文件的内容",
            &schema,
            &[],
            "读取一下 /home/user/test.py",
        );

        assert!(prompt.contains("参数提取助手"));
        assert!(prompt.contains("读取文件"));
        assert!(prompt.contains("读取本地文本文件的内容"));
        assert!(prompt.contains("path"));
        assert!(prompt.contains("读取一下 /home/user/test.py"));
        assert!(prompt.contains("输出:"));
    }

    #[test]
    fn test_build_extraction_prompt_with_examples() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" }
            }
        });

        let examples = vec![
            SkillExample {
                name: "01_basic".to_string(),
                content: r#"用户输入: "读取 /etc/hosts"
输出: {"path": "/etc/hosts"}"#
                    .to_string(),
            },
            SkillExample {
                name: "02_advanced".to_string(),
                content: r#"用户输入: "读 README.md 前 50 行"
输出: {"path": "README.md", "max_lines": 50}"#
                    .to_string(),
            },
        ];

        let prompt = build_extraction_prompt(
            "读取文件",
            "读取文件内容",
            &schema,
            &examples,
            "读取 config.yaml",
        );

        assert!(prompt.contains("示例 1:"));
        assert!(prompt.contains("示例 2:"));
        assert!(prompt.contains("/etc/hosts"));
        assert!(prompt.contains("README.md"));
    }

    #[test]
    fn test_build_correction_prompt() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" }
            }
        });

        let prompt = build_correction_prompt(
            "读取文件",
            "读取文件内容",
            &schema,
            &[],
            "读取 test.py",
            "not valid json",
            &["输出不是有效的 JSON".to_string()],
        );

        assert!(prompt.contains("上次的输出"));
        assert!(prompt.contains("not valid json"));
        assert!(prompt.contains("输出不是有效的 JSON"));
        assert!(prompt.contains("请修正"));
    }

    #[test]
    fn test_format_schema_empty() {
        let schema = serde_json::json!({});
        let formatted = format_schema(&schema);
        assert_eq!(formatted, "{}");
    }

    #[test]
    fn test_format_schema_pretty() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"]
        });
        let formatted = format_schema(&schema);
        assert!(formatted.contains("\n"));
        assert!(formatted.contains("\"type\": \"object\""));
    }

    #[test]
    fn test_format_examples_empty() {
        let formatted = format_examples(&[], 3);
        assert!(formatted.is_empty());
    }

    #[test]
    fn test_format_examples_max() {
        let examples: Vec<SkillExample> = (0..5)
            .map(|i| SkillExample {
                name: format!("{i:02}_example"),
                content: format!("用户输入: \"test{i}\"\n输出: {{\"val\": {i}}}"),
            })
            .collect();

        let formatted = format_examples(&examples, 3);
        assert!(formatted.contains("示例 1:"));
        assert!(formatted.contains("示例 2:"));
        assert!(formatted.contains("示例 3:"));
        assert!(!formatted.contains("示例 4:"));
    }

    #[test]
    fn test_format_missing_field_question_with_desc() {
        let question = format_missing_field_question("path", "文件路径", "string");
        assert!(question.contains("path"));
        assert!(question.contains("文件路径"));
        assert!(question.contains("string"));
    }

    #[test]
    fn test_format_missing_field_question_without_desc() {
        let question = format_missing_field_question("name", "", "string");
        assert!(question.contains("name"));
        assert!(question.contains("string"));
    }
}
