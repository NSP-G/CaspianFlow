//! Slot Filler IPC commands.
//!
//! These functions are designed to be annotated with `#[tauri::command]`
//! once the Tauri runtime is integrated. For now they are plain async
//! functions that can be called from Rust or wrapped by the frontend bridge.

use std::sync::Arc;

use serde_json::Value;

use crate::router::slot_filler::{MissingField, SlotFillResult, SlotFiller, SlotFillerConfig};
use crate::skill::examples::SkillExample;
use crate::skill::schema::Skill;
use crate::types::SlotFillingResult;

/// Fill slots for a matched skill given user input.
///
/// This is the main entry point for parameter extraction.
/// Returns `SlotFillResult` which may be `Success`, `NeedsUserInput`, or `Failed`.
pub async fn fill_slots(
    filler: &SlotFiller,
    skill: &Skill,
    user_input: &str,
    examples: &[SkillExample],
) -> SlotFillResult {
    filler.fill_slots(skill, user_input, examples).await
}

/// Fill missing parameters after user provides answers.
///
/// Called when `fill_slots` returned `NeedsUserInput`. The caller collects
/// user answers for the missing fields and passes them here.
pub fn fill_missing_params(
    filler: &SlotFiller,
    skill: &Skill,
    partial_params: Value,
    user_answers: &[(String, Value)],
) -> SlotFillingResult<Value> {
    filler.fill_missing_params(skill, partial_params, user_answers)
}

/// Create a new slot filler with a single LLM provider.
pub fn create_slot_filler(
    provider: Arc<dyn crate::router::slot_filler::LlmProvider>,
) -> SlotFiller {
    SlotFiller::new(provider)
}

/// Create a new slot filler with separate small and large providers.
pub fn create_slot_filler_with_providers(
    small: Arc<dyn crate::router::slot_filler::LlmProvider>,
    large: Arc<dyn crate::router::slot_filler::LlmProvider>,
) -> SlotFiller {
    SlotFiller::with_providers(small, large)
}

/// Create a slot filler with custom configuration.
pub fn create_slot_filler_with_config(
    provider: Arc<dyn crate::router::slot_filler::LlmProvider>,
    config: SlotFillerConfig,
) -> SlotFiller {
    SlotFiller::new(provider).with_config(config)
}

/// Check if a slot fill result is a success.
pub fn is_success(result: &SlotFillResult) -> bool {
    matches!(result, SlotFillResult::Success { .. })
}

/// Check if a slot fill result needs user input.
pub fn needs_user_input(result: &SlotFillResult) -> bool {
    matches!(result, SlotFillResult::NeedsUserInput { .. })
}

/// Check if a slot fill result is a failure.
pub fn is_failed(result: &SlotFillResult) -> bool {
    matches!(result, SlotFillResult::Failed { .. })
}

/// Get the extracted parameters from a successful result.
///
/// Returns `None` if the result is not `Success`.
pub fn get_params(result: &SlotFillResult) -> Option<&Value> {
    match result {
        SlotFillResult::Success { params, .. } => Some(params),
        _ => None,
    }
}

/// Get the missing fields from a `NeedsUserInput` result.
///
/// Returns `None` if the result is not `NeedsUserInput`.
pub fn get_missing_fields(result: &SlotFillResult) -> Option<&[MissingField]> {
    match result {
        SlotFillResult::NeedsUserInput { missing_fields, .. } => Some(missing_fields),
        _ => None,
    }
}

/// Get the number of LLM calls made during slot filling.
pub fn get_attempts(result: &SlotFillResult) -> usize {
    match result {
        SlotFillResult::Success { attempts, .. } => *attempts,
        SlotFillResult::NeedsUserInput { attempts, .. } => *attempts,
        SlotFillResult::Failed { attempts, .. } => *attempts,
    }
}

/// Format missing fields as user-facing questions.
///
/// Returns a list of (field_name, question) pairs.
pub fn format_missing_field_questions(missing_fields: &[MissingField]) -> Vec<(String, String)> {
    missing_fields
        .iter()
        .map(|f| {
            let question = crate::router::prompt_templates::format_missing_field_question(
                &f.name,
                &f.description,
                &f.param_type,
            );
            (f.name.clone(), question)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::slot_filler::MockLlmProvider;
    use crate::skill::schema::{Skill, SkillRuntime, SkillRuntimeType};
    use std::path::PathBuf;

    fn make_skill(name: &str, schema: Value) -> Skill {
        Skill {
            mcp: None,
            schema_version: "1.0".to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Test skill {name}"),
            category: "test".to_string(),
            trigger_phrases: vec!["test".to_string()],
            runtime: SkillRuntime {
                runtime_type: SkillRuntimeType::Python,
                entry: "script.py".to_string(),
                timeout: 30,
                memory_limit_mb: 256,
            },
            input_schema: schema,
            output_schema: serde_json::json!({}),
            permissions: Default::default(),
            tags: vec![],
            author: "Test".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: PathBuf::from(format!("/skills/{name}")),
        }
    }

    fn read_file_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string", "description": "文件路径" }
            }
        })
    }

    #[tokio::test]
    async fn test_fill_slots_command_success() {
        let provider = Arc::new(MockLlmProvider::single(
            r#"{"path": "/test.py"}"#.to_string(),
        ));
        let filler = create_slot_filler(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = fill_slots(&filler, &skill, "读取 /test.py", &[]).await;

        assert!(is_success(&result));
        assert!(!needs_user_input(&result));
        assert!(!is_failed(&result));

        let params = get_params(&result).unwrap();
        assert_eq!(params["path"], "/test.py");

        assert_eq!(get_attempts(&result), 1);
    }

    #[tokio::test]
    async fn test_fill_slots_command_needs_input() {
        let provider = Arc::new(MockLlmProvider::new(vec![
            r#"{"max_lines": 50}"#.to_string(),
            r#"{"max_lines": 50}"#.to_string(),
            r#"{"max_lines": 50}"#.to_string(),
        ]));
        let filler = create_slot_filler(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = fill_slots(&filler, &skill, "读 50 行", &[]).await;

        assert!(needs_user_input(&result));
        let missing = get_missing_fields(&result).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "path");
    }

    #[tokio::test]
    async fn test_fill_slots_command_failed() {
        let provider = Arc::new(MockLlmProvider::new(vec![
            "bad".to_string(),
            "bad".to_string(),
            "bad".to_string(),
        ]));
        let filler = create_slot_filler(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = fill_slots(&filler, &skill, "test", &[]).await;

        assert!(is_failed(&result));
        assert_eq!(get_attempts(&result), 3);
    }

    #[test]
    fn test_fill_missing_params_command() {
        let provider = Arc::new(MockLlmProvider::single("{}".to_string()));
        let filler = create_slot_filler(provider);
        let skill = make_skill("read_file", read_file_schema());

        let partial = serde_json::json!({});
        let answers = vec![("path".to_string(), serde_json::json!("/test.py"))];

        let result = fill_missing_params(&filler, &skill, partial, &answers);

        assert!(result.is_ok());
        assert_eq!(result.unwrap()["path"], "/test.py");
    }

    #[test]
    fn test_format_missing_field_questions() {
        let fields = vec![MissingField {
            name: "path".to_string(),
            description: "文件路径".to_string(),
            param_type: "string".to_string(),
            default: None,
        }];

        let questions = format_missing_field_questions(&fields);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].0, "path");
        assert!(questions[0].1.contains("path"));
        assert!(questions[0].1.contains("文件路径"));
    }

    #[test]
    fn test_get_params_none_for_non_success() {
        let result = SlotFillResult::Failed {
            error: "test".to_string(),
            raw_output: "test".to_string(),
            attempts: 3,
        };
        assert!(get_params(&result).is_none());
    }

    #[test]
    fn test_get_missing_fields_none_for_success() {
        let result = SlotFillResult::Success {
            params: serde_json::json!({}),
            attempts: 1,
        };
        assert!(get_missing_fields(&result).is_none());
    }
}
