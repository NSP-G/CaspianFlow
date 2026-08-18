//! Slot Filler — extracts skill parameters from natural language input.
//!
//! The slot filler is the downstream component of the semantic router.
//! Given a matched skill and user input, it:
//!
//! 1. Assembles a prompt with the skill's `input_schema` and few-shot examples
//! 2. Calls an LLM provider to extract parameters as JSON
//! 3. Tolerantly parses the LLM output (4-layer fallback)
//! 4. Validates the extracted JSON against `input_schema`
//! 5. Retries with correction feedback on failure (up to 3 attempts)
//! 6. Fills in default values from the schema
//! 7. Returns `Success`, `NeedsUserInput`, or `Failed`

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::skill::schema::Skill;
use crate::types::{LlmError, LlmResult, SlotFillingError, SlotFillingResult};

use super::prompt_templates;

// ---------------------------------------------------------------------------
// LLM Provider trait (P23 concept-first)
// ---------------------------------------------------------------------------

/// The size class of an LLM — determines cost and latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSize {
    /// Small model: local 7B or cloud mini model — fast, cheap.
    Small,
    /// Large model: GPT-4o, DeepSeek Chat, etc. — higher quality, slower.
    Large,
}

impl std::fmt::Display for ModelSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Small => write!(f, "small"),
            Self::Large => write!(f, "large"),
        }
    }
}

/// The LLM provider abstraction.
///
/// P13 defines this trait; P23 (Model Adapter) will provide concrete
/// implementations for DeepSeek, OpenAI, Ollama, etc.
///
/// The provider is responsible for:
/// - Sending the prompt to the model
/// - Returning the raw text response
///
/// The provider is NOT responsible for:
/// - JSON parsing or validation
/// - Schema awareness
/// - Retry logic
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a text completion for the given prompt.
    async fn generate(&self, prompt: &str) -> LlmResult<String>;

    /// The model's display name (e.g. "deepseek-chat").
    fn model_name(&self) -> &str;

    /// The model's size class.
    fn model_size(&self) -> ModelSize;
}

// ---------------------------------------------------------------------------
// MockLlmProvider — for testing without network dependencies
// ---------------------------------------------------------------------------

/// A mock LLM provider with a programmable response queue.
///
/// Responses are returned in FIFO order. If the queue is exhausted,
/// returns `InvalidResponse` error.
///
/// This design supports multi-round retry scenarios: push a bad JSON
/// response first, then a good one, to test the retry mechanism.
pub struct MockLlmProvider {
    responses: Mutex<Vec<String>>,
    model_name: String,
    model_size: ModelSize,
    call_count: Mutex<usize>,
}

impl MockLlmProvider {
    /// Create a new mock provider with the given responses.
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Mutex::new(responses),
            model_name: "mock-llm".to_string(),
            model_size: ModelSize::Small,
            call_count: Mutex::new(0),
        }
    }

    /// Create a mock provider that always returns the same response.
    pub fn single(response: String) -> Self {
        Self::new(vec![response])
    }

    /// Create a mock provider with a specific model size.
    pub fn with_size(mut self, size: ModelSize) -> Self {
        self.model_size = size;
        self
    }

    /// Create a mock provider with a specific model name.
    pub fn with_name(mut self, name: &str) -> Self {
        self.model_name = name.to_string();
        self
    }

    /// Get the number of times `generate()` was called.
    pub fn call_count(&self) -> usize {
        *self.call_count.lock()
    }

    /// Push an additional response to the queue.
    pub fn push_response(&self, response: String) {
        self.responses.lock().push(response);
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn generate(&self, _prompt: &str) -> LlmResult<String> {
        let mut count = self.call_count.lock();
        *count += 1;
        drop(count);

        let mut responses = self.responses.lock();
        if responses.is_empty() {
            return Err(LlmError::InvalidResponse(
                "mock response queue exhausted".to_string(),
            ));
        }
        Ok(responses.remove(0))
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn model_size(&self) -> ModelSize {
        self.model_size
    }
}

// ---------------------------------------------------------------------------
// Tolerant JSON extraction (4-layer fallback)
// ---------------------------------------------------------------------------

/// Extract a JSON object from potentially noisy LLM output.
///
/// Tries four strategies in order:
/// 1. Direct `serde_json::from_str` parse
/// 2. Extract ```json ... ``` code block
/// 3. Find the outermost balanced `{ ... }` substring
/// 4. Fail with `JsonParseFailed`
pub fn extract_json(raw: &str) -> SlotFillingResult<Value> {
    // Layer 1: direct parse
    if let Ok(val) = serde_json::from_str::<Value>(raw.trim()) {
        if val.is_object() {
            return Ok(val);
        }
    }

    // Layer 2: markdown code block ```json ... ``` or ``` ... ```
    if let Some(json_str) = extract_code_block(raw) {
        if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
            if val.is_object() {
                return Ok(val);
            }
        }
    }

    // Layer 3: outermost balanced braces
    if let Some(json_str) = extract_balanced_braces(raw) {
        if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
            if val.is_object() {
                return Ok(val);
            }
        }
    }

    // Layer 4: failure
    Err(SlotFillingError::JsonParseFailed {
        raw_output: raw.to_string(),
    })
}

/// Extract content from a markdown code block.
fn extract_code_block(raw: &str) -> Option<String> {
    // Look for ```json ... ``` or ``` ... ```
    let start_marker = "```";
    let start_idx = raw.find(start_marker)?;
    let after_start = &raw[start_idx + start_marker.len()..];

    // Skip optional "json" language tag
    let content_start = if after_start.trim_start().starts_with("json") {
        after_start
            .trim_start()
            .strip_prefix("json")
            .unwrap_or(after_start)
    } else {
        after_start
    };

    // Skip leading whitespace/newline
    let content_start = content_start.trim_start();

    // Find closing ```
    let end_idx = content_start.find(start_marker)?;
    Some(content_start[..end_idx].trim().to_string())
}

/// Extract the outermost balanced `{ ... }` substring.
///
/// Tracks brace depth, handling string literals (ignores braces inside strings).
fn extract_balanced_braces(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, &b) in bytes.iter().enumerate() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if b == b'\\' && in_string {
            escape_next = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == b'{' {
            if depth == 0 {
                start = Some(i);
            }
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                if let Some(s) = start {
                    return Some(raw[s..=i].to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Lightweight Schema validator (zero-dependency)
// ---------------------------------------------------------------------------

/// A single schema validation error.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaValidationError {
    /// Dot-separated path to the field (e.g. "address.city").
    pub field_path: String,
    /// Human-readable error description.
    pub message: String,
}

impl std::fmt::Display for SchemaValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.field_path, self.message)
    }
}

/// The set of JSON Schema keywords this validator supports.
pub const SUPPORTED_KEYWORDS: &[&str] = &[
    "type",
    "required",
    "properties",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "enum",
    "items",
    "default",
    "description",
];

/// Validate a JSON value against a JSON Schema.
///
/// This is a lightweight, zero-dependency validator covering the most
/// common JSON Schema keywords. For unsupported keywords, a warning is
/// emitted but validation does not fail.
///
/// # Supported keywords
/// - `type`: string, integer, number, boolean, object, array, null
/// - `required`: list of mandatory property names
/// - `properties`: per-property sub-schemas (recursive)
/// - `minimum` / `maximum`: numeric bounds
/// - `minLength` / `maxLength`: string length bounds
/// - `enum`: allowed values list
/// - `items`: array element sub-schema (recursive)
/// - `default`: ignored during validation (used in `apply_defaults`)
pub fn validate_against_schema(value: &Value, schema: &Value) -> Vec<SchemaValidationError> {
    let mut errors = Vec::new();
    validate_value(value, schema, "$root", &mut errors);
    errors
}

/// Check if all values in the schema use only supported keywords.
///
/// Returns a list of unsupported keywords found (with paths).
pub fn check_schema_keywords(schema: &Value) -> Vec<(String, String)> {
    let mut unsupported = Vec::new();
    check_keywords_recursive(schema, "$root", &mut unsupported);
    unsupported
}

fn check_keywords_recursive(schema: &Value, path: &str, unsupported: &mut Vec<(String, String)>) {
    if let Some(obj) = schema.as_object() {
        for (key, val) in obj {
            match key.as_str() {
                "type" | "required" | "properties" | "minimum" | "maximum" | "minLength"
                | "maxLength" | "enum" | "items" | "default" | "description" => {
                    // Recurse into properties and items
                    if key == "properties" {
                        if let Some(props) = val.as_object() {
                            for (prop_name, prop_schema) in props {
                                check_keywords_recursive(
                                    prop_schema,
                                    &format!("{path}.{prop_name}"),
                                    unsupported,
                                );
                            }
                        }
                    } else if key == "items" {
                        check_keywords_recursive(val, &format!("{path}[]"), unsupported);
                    }
                }
                _ => {
                    unsupported.push((path.to_string(), key.clone()));
                }
            }
        }
    }
}

fn validate_value(
    value: &Value,
    schema: &Value,
    path: &str,
    errors: &mut Vec<SchemaValidationError>,
) {
    let schema_obj = match schema.as_object() {
        Some(o) => o,
        None => return, // No schema to validate against
    };

    // Check type
    if let Some(schema_type) = schema_obj.get("type").and_then(|v| v.as_str()) {
        if !check_type(value, schema_type) {
            errors.push(SchemaValidationError {
                field_path: path.to_string(),
                message: format!(
                    "expected type `{schema_type}`, got `{}`",
                    json_type_name(value)
                ),
            });
            return; // Type mismatch → no point checking further constraints
        }
    }

    // Check enum
    if let Some(enum_vals) = schema_obj.get("enum").and_then(|v| v.as_array()) {
        if !enum_vals.iter().any(|e| e == value) {
            errors.push(SchemaValidationError {
                field_path: path.to_string(),
                message: format!("value not in allowed enum values: {:?}", enum_vals),
            });
        }
    }

    // Type-specific validations
    match value {
        Value::Object(obj) => {
            validate_object(obj, schema_obj, path, errors);
        }
        Value::Array(arr) => {
            validate_array(arr, schema_obj, path, errors);
        }
        Value::Number(num) => {
            validate_number(num, schema_obj, path, errors);
        }
        Value::String(s) => {
            validate_string(s, schema_obj, path, errors);
        }
        _ => {}
    }
}

fn validate_object(
    obj: &serde_json::Map<String, Value>,
    schema: &serde_json::Map<String, Value>,
    path: &str,
    errors: &mut Vec<SchemaValidationError>,
) {
    // Check required fields
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        for req in required {
            if let Some(req_name) = req.as_str() {
                if !obj.contains_key(req_name) {
                    errors.push(SchemaValidationError {
                        field_path: format!("{path}.{req_name}"),
                        message: "required field is missing".to_string(),
                    });
                }
            }
        }
    }

    // Validate individual properties
    if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
        for (prop_name, prop_value) in obj {
            if let Some(prop_schema) = properties.get(prop_name) {
                let prop_path = format!("{path}.{prop_name}");
                validate_value(prop_value, prop_schema, &prop_path, errors);
            }
            // Unknown properties are allowed (additionalProperties not enforced)
        }
    }
}

fn validate_array(
    arr: &[Value],
    schema: &serde_json::Map<String, Value>,
    path: &str,
    errors: &mut Vec<SchemaValidationError>,
) {
    if let Some(items_schema) = schema.get("items") {
        for (i, item) in arr.iter().enumerate() {
            let item_path = format!("{path}[{i}]");
            validate_value(item, items_schema, &item_path, errors);
        }
    }
}

fn validate_number(
    num: &serde_json::Number,
    schema: &serde_json::Map<String, Value>,
    path: &str,
    errors: &mut Vec<SchemaValidationError>,
) {
    let val = if num.is_f64() {
        num.as_f64().unwrap_or(f64::NAN)
    } else if num.is_i64() {
        num.as_i64().map(|v| v as f64).unwrap_or(f64::NAN)
    } else if num.is_u64() {
        num.as_u64().map(|v| v as f64).unwrap_or(f64::NAN)
    } else {
        f64::NAN
    };

    if let Some(min) = schema.get("minimum").and_then(|v| v.as_f64()) {
        if val < min {
            errors.push(SchemaValidationError {
                field_path: path.to_string(),
                message: format!("value {val} is below minimum {min}"),
            });
        }
    }

    if let Some(max) = schema.get("maximum").and_then(|v| v.as_f64()) {
        if val > max {
            errors.push(SchemaValidationError {
                field_path: path.to_string(),
                message: format!("value {val} exceeds maximum {max}"),
            });
        }
    }
}

fn validate_string(
    s: &str,
    schema: &serde_json::Map<String, Value>,
    path: &str,
    errors: &mut Vec<SchemaValidationError>,
) {
    if let Some(min_len) = schema.get("minLength").and_then(|v| v.as_u64()) {
        let len = s.chars().count() as u64;
        if len < min_len {
            errors.push(SchemaValidationError {
                field_path: path.to_string(),
                message: format!("string length {len} is below minLength {min_len}"),
            });
        }
    }

    if let Some(max_len) = schema.get("maxLength").and_then(|v| v.as_u64()) {
        let len = s.chars().count() as u64;
        if len > max_len {
            errors.push(SchemaValidationError {
                field_path: path.to_string(),
                message: format!("string length {len} exceeds maxLength {max_len}"),
            });
        }
    }
}

/// Check if a JSON value matches the expected schema type string.
fn check_type(value: &Value, schema_type: &str) -> bool {
    match schema_type {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => true, // Unknown type → don't fail (lenient)
    }
}

/// Get the JSON type name of a value for error messages.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => {
            if value.is_i64() || value.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// Default value application
// ---------------------------------------------------------------------------

/// Fill in default values from the schema for missing properties.
///
/// Walks `schema.properties` and adds `default` values for any property
/// not present in `value`. Operates recursively on nested objects.
pub fn apply_defaults(value: &mut Value, schema: &Value) {
    let schema_obj = match schema.as_object() {
        Some(o) => o,
        None => return,
    };

    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    if let Some(properties) = schema_obj.get("properties").and_then(|v| v.as_object()) {
        for (prop_name, prop_schema) in properties {
            // If property is missing and has a default → add it
            if !obj.contains_key(prop_name) {
                if let Some(default_val) = prop_schema.get("default") {
                    obj.insert(prop_name.clone(), default_val.clone());
                }
            }

            // Recurse into nested objects
            if let Some(val) = obj.get_mut(prop_name) {
                if val.is_object() {
                    apply_defaults(val, prop_schema);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Schema complexity assessment (model auto-selection)
// ---------------------------------------------------------------------------

/// The complexity class of a JSON Schema — determines model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaComplexity {
    /// Simple: 1-2 required fields, no nesting → small model.
    Simple,
    /// Complex: 3+ required fields, or nesting, or arrays of objects → large model.
    Complex,
}

impl std::fmt::Display for SchemaComplexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simple => write!(f, "simple"),
            Self::Complex => write!(f, "complex"),
        }
    }
}

/// Assess the complexity of a JSON Schema.
///
/// Heuristics:
/// - Count required fields (> 2 → complex)
/// - Check for nested objects → complex
/// - Check for arrays of objects → complex
/// - Total property count (> 5 → complex)
pub fn assess_complexity(schema: &Value) -> SchemaComplexity {
    let schema_obj = match schema.as_object() {
        Some(o) => o,
        None => return SchemaComplexity::Simple,
    };

    // Count required fields
    let required_count = schema_obj
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    if required_count > 2 {
        return SchemaComplexity::Complex;
    }

    // Check properties for nesting
    if let Some(properties) = schema_obj.get("properties").and_then(|v| v.as_object()) {
        let total_props = properties.len();
        if total_props > 5 {
            return SchemaComplexity::Complex;
        }

        for (_, prop_schema) in properties {
            // Nested object?
            if prop_schema.get("type").and_then(|v| v.as_str()) == Some("object") {
                return SchemaComplexity::Complex;
            }
            // Array of objects?
            if prop_schema.get("type").and_then(|v| v.as_str()) == Some("array") {
                if let Some(items) = prop_schema.get("items") {
                    if items.get("type").and_then(|v| v.as_str()) == Some("object") {
                        return SchemaComplexity::Complex;
                    }
                }
            }
        }
    }

    SchemaComplexity::Simple
}

/// Select the appropriate model size based on schema complexity.
pub fn select_model_size(complexity: SchemaComplexity) -> ModelSize {
    match complexity {
        SchemaComplexity::Simple => ModelSize::Small,
        SchemaComplexity::Complex => ModelSize::Large,
    }
}

// ---------------------------------------------------------------------------
// Slot fill result types
// ---------------------------------------------------------------------------

/// A field that is missing and needs user input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingField {
    /// The field name from the schema.
    pub name: String,
    /// Human-readable description from the schema.
    pub description: String,
    /// The expected type (from schema).
    pub param_type: String,
    /// Default value from schema, if any.
    pub default: Option<Value>,
}

/// The outcome of a slot-filling operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SlotFillResult {
    /// Parameters successfully extracted and validated.
    Success {
        /// The extracted parameters as a JSON object.
        params: Value,
        /// Number of LLM calls made (1 = first try, 2 = one retry, etc.).
        attempts: usize,
    },
    /// Some required fields are missing — ask the user for input.
    NeedsUserInput {
        /// List of missing required fields.
        missing_fields: Vec<MissingField>,
        /// Partially extracted params (valid but incomplete).
        partial_params: Value,
        /// Number of LLM calls made.
        attempts: usize,
    },
    /// Extraction failed after all retries.
    Failed {
        /// The error that caused the failure.
        error: String,
        /// The last raw LLM output.
        raw_output: String,
        /// Number of LLM calls made.
        attempts: usize,
    },
}

// ---------------------------------------------------------------------------
// SlotFiller — the core engine
// ---------------------------------------------------------------------------

/// Maximum retry attempts (total attempts = 1 + MAX_RETRIES).
const MAX_RETRIES: usize = 2;

/// Configuration for the slot filler.
#[derive(Debug, Clone)]
pub struct SlotFillerConfig {
    /// Maximum retries on validation failure (default: 2, total 3 attempts).
    pub max_retries: usize,
    /// Whether to use model auto-selection based on schema complexity.
    pub auto_select_model: bool,
}

impl Default for SlotFillerConfig {
    fn default() -> Self {
        Self {
            max_retries: MAX_RETRIES,
            auto_select_model: true,
        }
    }
}

/// The slot filler engine.
///
/// Owns references to LLM providers (small and large). The small provider
/// is used by default; the large provider is used when schema complexity
/// warrants it (if `auto_select_model` is enabled).
pub struct SlotFiller {
    small_provider: Arc<dyn LlmProvider>,
    large_provider: Option<Arc<dyn LlmProvider>>,
    config: SlotFillerConfig,
}

impl SlotFiller {
    /// Create a new slot filler with a single provider (used for both sizes).
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            small_provider: provider.clone(),
            large_provider: Some(provider),
            config: SlotFillerConfig::default(),
        }
    }

    /// Create a new slot filler with separate small and large providers.
    pub fn with_providers(small: Arc<dyn LlmProvider>, large: Arc<dyn LlmProvider>) -> Self {
        Self {
            small_provider: small,
            large_provider: Some(large),
            config: SlotFillerConfig::default(),
        }
    }

    /// Set a custom configuration.
    pub fn with_config(mut self, config: SlotFillerConfig) -> Self {
        self.config = config;
        self
    }

    /// Extract parameters from user input for the given skill.
    ///
    /// This is the main entry point. It:
    /// 1. Assembles the prompt
    /// 2. Calls the LLM
    /// 3. Parses and validates the output
    /// 4. Retries on failure
    /// 5. Returns the result
    pub async fn fill_slots(
        &self,
        skill: &Skill,
        user_input: &str,
        examples: &[crate::skill::examples::SkillExample],
    ) -> SlotFillResult {
        // Check for empty schema
        if skill.input_schema.is_null()
            || skill
                .input_schema
                .as_object()
                .map(|o| o.is_empty())
                .unwrap_or(true)
        {
            return SlotFillResult::Failed {
                error: SlotFillingError::EmptySchema.to_string(),
                raw_output: String::new(),
                attempts: 0,
            };
        }

        // Select provider based on schema complexity
        let provider = self.select_provider(&skill.input_schema);

        // Build the initial prompt
        let prompt = prompt_templates::build_extraction_prompt(
            &skill.display_name,
            &skill.description,
            &skill.input_schema,
            examples,
            user_input,
        );

        let max_attempts = self.config.max_retries + 1;
        let mut last_errors: Vec<String> = Vec::new();
        let mut last_raw_output = String::new();
        let mut current_prompt = prompt;

        for attempt in 1..=max_attempts {
            tracing::info!(
                skill = %skill.name,
                attempt,
                max_attempts,
                "slot filling attempt"
            );

            // Call the LLM
            let raw_output = match provider.generate(&current_prompt).await {
                Ok(text) => text,
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "LLM call failed");
                    last_errors.push(format!("LLM error: {e}"));
                    last_raw_output.clear();
                    continue;
                }
            };
            last_raw_output = raw_output.clone();

            // Parse JSON from output
            let parsed = match extract_json(&raw_output) {
                Ok(val) => val,
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "JSON extraction failed");
                    last_errors.push(e.to_string());
                    // Build correction prompt for retry
                    if attempt < max_attempts {
                        current_prompt = prompt_templates::build_correction_prompt(
                            &skill.display_name,
                            &skill.description,
                            &skill.input_schema,
                            examples,
                            user_input,
                            &raw_output,
                            &[
                                "输出不是有效的 JSON，请确保只输出 JSON 对象，不要包含其他文本"
                                    .to_string(),
                            ],
                        );
                    }
                    continue;
                }
            };

            // Validate against schema
            let errors = validate_against_schema(&parsed, &skill.input_schema);

            if errors.is_empty() {
                // Success! Apply defaults and return
                let mut result = parsed;
                apply_defaults(&mut result, &skill.input_schema);
                tracing::info!(
                    skill = %skill.name,
                    attempt,
                    "slot filling succeeded"
                );
                return SlotFillResult::Success {
                    params: result,
                    attempts: attempt,
                };
            }

            // Validation failed
            let error_strs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            last_errors = error_strs.clone();

            tracing::warn!(
                attempt,
                errors = ?error_strs,
                "schema validation failed"
            );

            // Build correction prompt for retry
            if attempt < max_attempts {
                current_prompt = prompt_templates::build_correction_prompt(
                    &skill.display_name,
                    &skill.description,
                    &skill.input_schema,
                    examples,
                    user_input,
                    &raw_output,
                    &error_strs,
                );
            }
        }

        // All attempts exhausted — check if we got valid JSON with missing fields
        // Only return NeedsUserInput if the LLM produced parseable JSON but
        // with missing required fields. If the output was never valid JSON,
        // return Failed instead.
        if let Ok(partial) = extract_json(&last_raw_output) {
            let missing = find_missing_required_fields(&skill.input_schema, &last_raw_output);
            if !missing.is_empty() {
                return SlotFillResult::NeedsUserInput {
                    missing_fields: missing,
                    partial_params: partial,
                    attempts: max_attempts,
                };
            }
        }

        SlotFillResult::Failed {
            error: SlotFillingError::MaxRetriesExceeded {
                max_retries: self.config.max_retries,
                attempts: max_attempts,
                last_errors,
                raw_output: last_raw_output.clone(),
            }
            .to_string(),
            raw_output: last_raw_output,
            attempts: max_attempts,
        }
    }

    /// Merge user-provided answers for missing fields with existing partial params.
    ///
    /// After a `NeedsUserInput` result, the user provides values for the
    /// missing fields. This function merges them and re-validates.
    pub fn fill_missing_params(
        &self,
        skill: &Skill,
        partial_params: Value,
        user_answers: &[(String, Value)],
    ) -> SlotFillingResult<Value> {
        let mut result = partial_params;

        // Merge user answers
        if let Some(obj) = result.as_object_mut() {
            for (key, val) in user_answers {
                obj.insert(key.clone(), val.clone());
            }
        } else {
            return Err(SlotFillingError::JsonParseFailed {
                raw_output: "partial_params is not a JSON object".to_string(),
            });
        }

        // Validate the merged result
        let errors = validate_against_schema(&result, &skill.input_schema);

        if !errors.is_empty() {
            let error_strs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            return Err(SlotFillingError::ValidationFailed {
                error_count: errors.len(),
                errors: error_strs.join("; "),
                raw_output: result.to_string(),
            });
        }

        // Apply defaults
        apply_defaults(&mut result, &skill.input_schema);

        Ok(result)
    }

    /// Select the appropriate LLM provider based on schema complexity.
    fn select_provider(&self, schema: &Value) -> &dyn LlmProvider {
        if !self.config.auto_select_model {
            return self.small_provider.as_ref();
        }

        let complexity = assess_complexity(schema);
        match complexity {
            SchemaComplexity::Simple => self.small_provider.as_ref(),
            SchemaComplexity::Complex => self
                .large_provider
                .as_ref()
                .map(|p| p.as_ref())
                .unwrap_or_else(|| self.small_provider.as_ref()),
        }
    }
}

impl std::fmt::Debug for SlotFiller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlotFiller")
            .field("small_model", &self.small_provider.model_name())
            .field(
                "large_model",
                &self
                    .large_provider
                    .as_ref()
                    .map(|p| p.model_name().to_string()),
            )
            .field("max_retries", &self.config.max_retries)
            .field("auto_select_model", &self.config.auto_select_model)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Helper: find missing required fields from schema
// ---------------------------------------------------------------------------

/// Find required fields from the schema that could not be extracted.
///
/// This is called after all retries fail, to determine if we should
/// ask the user for input (NeedsUserInput) or report a hard failure.
fn find_missing_required_fields(schema: &Value, raw_output: &str) -> Vec<MissingField> {
    let schema_obj = match schema.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    // Try to parse the last output to see what we got
    let parsed = extract_json(raw_output)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    let empty_required: Vec<Value> = Vec::new();
    let required = schema_obj
        .get("required")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_required);

    let properties = schema_obj.get("properties").and_then(|v| v.as_object());

    let mut missing = Vec::new();

    for req in required {
        if let Some(field_name) = req.as_str() {
            if parsed.contains_key(field_name) {
                continue;
            }

            let (description, param_type, default) = properties
                .and_then(|p| p.get(field_name))
                .map(|prop| {
                    let desc = prop
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ptype = prop
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("any")
                        .to_string();
                    let def = prop.get("default").cloned();
                    (desc, ptype, def)
                })
                .unwrap_or_default();

            missing.push(MissingField {
                name: field_name.to_string(),
                description,
                param_type,
                default,
            });
        }
    }

    missing
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::examples::SkillExample;
    use crate::skill::schema::{Skill, SkillRuntime, SkillRuntimeType};
    use std::path::PathBuf;

    // --- Test helpers ---

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
                "path": { "type": "string", "description": "文件路径" },
                "max_lines": { "type": "integer", "description": "最大读取行数", "minimum": 1, "maximum": 10000 }
            }
        })
    }

    // =====================
    // JSON extraction tests
    // =====================

    #[test]
    fn test_extract_json_pure() {
        let result = extract_json(r#"{"path": "/test.py"}"#).unwrap();
        assert_eq!(result["path"], "/test.py");
    }

    #[test]
    fn test_extract_json_markdown_block() {
        let input = "好的，结果如下：\n```json\n{\"path\": \"/test.py\"}\n```\n希望有帮助";
        let result = extract_json(input).unwrap();
        assert_eq!(result["path"], "/test.py");
    }

    #[test]
    fn test_extract_json_code_block_no_lang() {
        let input = "```\n{\"path\": \"/test.py\"}\n```";
        let result = extract_json(input).unwrap();
        assert_eq!(result["path"], "/test.py");
    }

    #[test]
    fn test_extract_json_with_prefix_suffix() {
        let input = "好的，结果是：{\"path\": \"/test.py\"} 希望有帮助";
        let result = extract_json(input).unwrap();
        assert_eq!(result["path"], "/test.py");
    }

    #[test]
    fn test_extract_json_nested_braces() {
        let input = r#"结果：{"config": {"path": "/test", "opts": {"verbose": true}}}"#;
        let result = extract_json(input).unwrap();
        assert_eq!(result["config"]["path"], "/test");
        assert_eq!(result["config"]["opts"]["verbose"], true);
    }

    #[test]
    fn test_extract_json_braces_in_string() {
        let input = r#"{"path": "/test/{name}.py"}"#;
        let result = extract_json(input).unwrap();
        assert_eq!(result["path"], "/test/{name}.py");
    }

    #[test]
    fn test_extract_json_invalid() {
        let result = extract_json("this is not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_json_empty() {
        let result = extract_json("");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_json_array_rejected() {
        // We only accept JSON objects, not arrays
        let result = extract_json("[1, 2, 3]");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_json_multiline() {
        let input = "{\n  \"path\": \"/test.py\",\n  \"max_lines\": 50\n}";
        let result = extract_json(input).unwrap();
        assert_eq!(result["path"], "/test.py");
        assert_eq!(result["max_lines"], 50);
    }

    // =====================
    // Schema validation tests
    // =====================

    #[test]
    fn test_validate_valid_object() {
        let schema = read_file_schema();
        let value = serde_json::json!({"path": "/test.py", "max_lines": 50});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.is_empty(), "should have no errors: {:?}", errors);
    }

    #[test]
    fn test_validate_missing_required() {
        let schema = read_file_schema();
        let value = serde_json::json!({"max_lines": 50});
        let errors = validate_against_schema(&value, &schema);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].field_path.contains("path"));
        assert!(errors[0].message.contains("missing"));
    }

    #[test]
    fn test_validate_wrong_type() {
        let schema = read_file_schema();
        let value = serde_json::json!({"path": 123});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors
            .iter()
            .any(|e| e.message.contains("expected type `string`")));
    }

    #[test]
    fn test_validate_integer_type() {
        let schema = read_file_schema();
        let value = serde_json::json!({"path": "/test.py", "max_lines": "fifty"});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors
            .iter()
            .any(|e| e.message.contains("expected type `integer`")));
    }

    #[test]
    fn test_validate_below_minimum() {
        let schema = read_file_schema();
        let value = serde_json::json!({"path": "/test.py", "max_lines": 0});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.iter().any(|e| e.message.contains("below minimum")));
    }

    #[test]
    fn test_validate_exceeds_maximum() {
        let schema = read_file_schema();
        let value = serde_json::json!({"path": "/test.py", "max_lines": 99999});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.iter().any(|e| e.message.contains("exceeds maximum")));
    }

    #[test]
    fn test_validate_enum() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "mode": { "type": "string", "enum": ["fast", "slow", "auto"] }
            }
        });
        let value = serde_json::json!({"mode": "invalid"});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.iter().any(|e| e.message.contains("enum")));
    }

    #[test]
    fn test_validate_enum_valid() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "mode": { "type": "string", "enum": ["fast", "slow", "auto"] }
            }
        });
        let value = serde_json::json!({"mode": "fast"});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_string_length() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "minLength": 3, "maxLength": 10 }
            }
        });

        let value = serde_json::json!({"name": "ab"});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.iter().any(|e| e.message.contains("below minLength")));

        let value = serde_json::json!({"name": "this_is_too_long"});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors
            .iter()
            .any(|e| e.message.contains("exceeds maxLength")));
    }

    #[test]
    fn test_validate_nested_object() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["config"],
            "properties": {
                "config": {
                    "type": "object",
                    "required": ["path"],
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }
        });

        // Missing nested required field
        let value = serde_json::json!({"config": {}});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.iter().any(|e| e.field_path.contains("config.path")));

        // Valid nested
        let value = serde_json::json!({"config": {"path": "/test"}});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_array_items() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        });

        let value = serde_json::json!({"files": ["a.txt", "b.txt", 123]});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors
            .iter()
            .any(|e| e.message.contains("expected type `string`")));
    }

    #[test]
    fn test_validate_unknown_properties_allowed() {
        let schema = read_file_schema();
        let value = serde_json::json!({"path": "/test.py", "extra": "unknown"});
        let errors = validate_against_schema(&value, &schema);
        assert!(errors.is_empty(), "unknown properties should be allowed");
    }

    // =====================
    // Default value tests
    // =====================

    #[test]
    fn test_apply_defaults_basic() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "encoding": { "type": "string", "default": "utf-8" }
            }
        });

        let mut value = serde_json::json!({"path": "/test.py"});
        apply_defaults(&mut value, &schema);

        assert_eq!(value["encoding"], "utf-8");
        assert_eq!(value["path"], "/test.py");
    }

    #[test]
    fn test_apply_defaults_does_not_override() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "encoding": { "type": "string", "default": "utf-8" }
            }
        });

        let mut value = serde_json::json!({"encoding": "gbk"});
        apply_defaults(&mut value, &schema);

        // Existing value should not be overridden
        assert_eq!(value["encoding"], "gbk");
    }

    #[test]
    fn test_apply_defaults_nested() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {
                        "verbose": { "type": "boolean", "default": false }
                    }
                }
            }
        });

        let mut value = serde_json::json!({"config": {}});
        apply_defaults(&mut value, &schema);

        assert_eq!(value["config"]["verbose"], false);
    }

    #[test]
    fn test_apply_defaults_no_defaults() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            }
        });

        let mut value = serde_json::json!({"path": "/test"});
        apply_defaults(&mut value, &schema);
        // No defaults in schema → no change
        assert_eq!(value, serde_json::json!({"path": "/test"}));
    }

    // =====================
    // Complexity assessment tests
    // =====================

    #[test]
    fn test_complexity_simple() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" }
            }
        });
        assert_eq!(assess_complexity(&schema), SchemaComplexity::Simple);
    }

    #[test]
    fn test_complexity_many_required() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["a", "b", "c"],
            "properties": {
                "a": { "type": "string" },
                "b": { "type": "string" },
                "c": { "type": "string" }
            }
        });
        assert_eq!(assess_complexity(&schema), SchemaComplexity::Complex);
    }

    #[test]
    fn test_complexity_nested_object() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["config"],
            "properties": {
                "config": { "type": "object" }
            }
        });
        assert_eq!(assess_complexity(&schema), SchemaComplexity::Complex);
    }

    #[test]
    fn test_complexity_array_of_objects() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": { "type": "object" }
                }
            }
        });
        assert_eq!(assess_complexity(&schema), SchemaComplexity::Complex);
    }

    #[test]
    fn test_complexity_many_properties() {
        let mut props = serde_json::Map::new();
        for i in 0..6 {
            props.insert(format!("field_{i}"), serde_json::json!({"type": "string"}));
        }
        let schema = serde_json::json!({
            "type": "object",
            "properties": props
        });
        assert_eq!(assess_complexity(&schema), SchemaComplexity::Complex);
    }

    #[test]
    fn test_select_model_size() {
        assert_eq!(
            select_model_size(SchemaComplexity::Simple),
            ModelSize::Small
        );
        assert_eq!(
            select_model_size(SchemaComplexity::Complex),
            ModelSize::Large
        );
    }

    // =====================
    // MockLlmProvider tests
    // =====================

    #[tokio::test]
    async fn test_mock_provider_single() {
        let provider = MockLlmProvider::single(r#"{"path": "/test"}"#.to_string());
        let result = provider.generate("prompt").await.unwrap();
        assert_eq!(result, r#"{"path": "/test"}"#);
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_provider_queue() {
        let provider = MockLlmProvider::new(vec![
            "bad json".to_string(),
            r#"{"path": "/test"}"#.to_string(),
        ]);

        let first = provider.generate("prompt").await.unwrap();
        assert_eq!(first, "bad json");

        let second = provider.generate("prompt").await.unwrap();
        assert_eq!(second, r#"{"path": "/test"}"#);

        assert_eq!(provider.call_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_provider_exhausted() {
        let provider = MockLlmProvider::new(vec![]);
        let result = provider.generate("prompt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_provider_with_size() {
        let provider = MockLlmProvider::single("test".to_string())
            .with_size(ModelSize::Large)
            .with_name("test-model");

        assert_eq!(provider.model_size(), ModelSize::Large);
        assert_eq!(provider.model_name(), "test-model");
    }

    // =====================
    // SlotFiller integration tests
    // =====================

    #[tokio::test]
    async fn test_fill_slots_success_first_try() {
        let provider = Arc::new(MockLlmProvider::single(
            r#"{"path": "/home/user/test.py"}"#.to_string(),
        ));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = filler
            .fill_slots(&skill, "读取 /home/user/test.py", &[])
            .await;

        match result {
            SlotFillResult::Success { params, attempts } => {
                assert_eq!(params["path"], "/home/user/test.py");
                assert_eq!(attempts, 1);
            }
            _ => panic!("expected Success, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_success_with_defaults() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" },
                "encoding": { "type": "string", "default": "utf-8" }
            }
        });

        let provider = Arc::new(MockLlmProvider::single(
            r#"{"path": "/test.py"}"#.to_string(),
        ));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", schema);

        let result = filler.fill_slots(&skill, "读取 /test.py", &[]).await;

        match result {
            SlotFillResult::Success { params, .. } => {
                assert_eq!(params["path"], "/test.py");
                assert_eq!(params["encoding"], "utf-8"); // default filled
            }
            _ => panic!("expected Success"),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_retry_success() {
        // First attempt: bad JSON, second attempt: valid JSON
        let provider = Arc::new(MockLlmProvider::new(vec![
            "this is not json".to_string(),
            r#"{"path": "/test.py"}"#.to_string(),
        ]));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = filler.fill_slots(&skill, "读取 /test.py", &[]).await;

        match result {
            SlotFillResult::Success { params, attempts } => {
                assert_eq!(params["path"], "/test.py");
                assert_eq!(attempts, 2, "should succeed on second attempt");
            }
            _ => panic!("expected Success after retry"),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_retry_type_error() {
        // First attempt: wrong type, second attempt: correct
        let provider = Arc::new(MockLlmProvider::new(vec![
            r#"{"path": 123}"#.to_string(),
            r#"{"path": "/test.py"}"#.to_string(),
        ]));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = filler.fill_slots(&skill, "读取 /test.py", &[]).await;

        match result {
            SlotFillResult::Success { params, attempts } => {
                assert_eq!(params["path"], "/test.py");
                assert_eq!(attempts, 2);
            }
            _ => panic!("expected Success after type correction"),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_all_attempts_fail() {
        let provider = Arc::new(MockLlmProvider::new(vec![
            "bad json 1".to_string(),
            "bad json 2".to_string(),
            "bad json 3".to_string(),
        ]));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = filler.fill_slots(&skill, "读取 /test.py", &[]).await;

        match result {
            SlotFillResult::Failed { attempts, .. } => {
                assert_eq!(attempts, 3, "should exhaust all 3 attempts");
            }
            _ => panic!("expected Failed after all retries"),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_needs_user_input() {
        // LLM returns JSON but missing required field
        let provider = Arc::new(MockLlmProvider::new(vec![
            r#"{"max_lines": 50}"#.to_string(),
            r#"{"max_lines": 50}"#.to_string(),
            r#"{"max_lines": 50}"#.to_string(),
        ]));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = filler.fill_slots(&skill, "读 50 行", &[]).await;

        match result {
            SlotFillResult::NeedsUserInput {
                missing_fields,
                partial_params,
                ..
            } => {
                assert_eq!(missing_fields.len(), 1);
                assert_eq!(missing_fields[0].name, "path");
                assert_eq!(partial_params["max_lines"], 50);
            }
            _ => panic!("expected NeedsUserInput, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_empty_schema() {
        let provider = Arc::new(MockLlmProvider::single("{}".to_string()));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("empty", serde_json::json!({}));

        let result = filler.fill_slots(&skill, "test", &[]).await;

        match result {
            SlotFillResult::Failed { error, .. } => {
                assert!(error.contains("empty") || error.contains("EmptySchema"));
            }
            _ => panic!("expected Failed for empty schema"),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_with_markdown_output() {
        let provider = Arc::new(MockLlmProvider::single(
            r#"好的，结果如下：
```json
{"path": "/test.py"}
```
"#
            .to_string(),
        ));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let result = filler.fill_slots(&skill, "读取 /test.py", &[]).await;

        match result {
            SlotFillResult::Success { params, .. } => {
                assert_eq!(params["path"], "/test.py");
            }
            _ => panic!("expected Success with markdown output"),
        }
    }

    #[tokio::test]
    async fn test_fill_slots_with_examples() {
        let provider = Arc::new(MockLlmProvider::single(
            r#"{"path": "/etc/hosts"}"#.to_string(),
        ));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let examples = vec![SkillExample {
            name: "01_basic".to_string(),
            content: r#"用户输入: "读取 /etc/passwd"
输出: {"path": "/etc/passwd"}"#
                .to_string(),
        }];

        let result = filler
            .fill_slots(&skill, "读取 /etc/hosts", &examples)
            .await;

        match result {
            SlotFillResult::Success { params, .. } => {
                assert_eq!(params["path"], "/etc/hosts");
            }
            _ => panic!("expected Success with examples"),
        }
    }

    // =====================
    // fill_missing_params tests
    // =====================

    #[test]
    fn test_fill_missing_params_success() {
        let provider = Arc::new(MockLlmProvider::single("{}".to_string()));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let partial = serde_json::json!({"max_lines": 50});
        let user_answers = vec![("path".to_string(), serde_json::json!("/test.py"))];

        let result = filler.fill_missing_params(&skill, partial, &user_answers);

        assert!(result.is_ok());
        let params = result.unwrap();
        assert_eq!(params["path"], "/test.py");
        assert_eq!(params["max_lines"], 50);
    }

    #[test]
    fn test_fill_missing_params_still_invalid() {
        let provider = Arc::new(MockLlmProvider::single("{}".to_string()));
        let filler = SlotFiller::new(provider);
        let skill = make_skill("read_file", read_file_schema());

        let partial = serde_json::json!({});
        // User provides wrong type
        let user_answers = vec![("path".to_string(), serde_json::json!(123))];

        let result = filler.fill_missing_params(&skill, partial, &user_answers);
        assert!(result.is_err());
    }

    // =====================
    // check_schema_keywords tests
    // =====================

    #[test]
    fn test_check_keywords_all_supported() {
        let schema = read_file_schema();
        let unsupported = check_schema_keywords(&schema);
        assert!(
            unsupported.is_empty(),
            "should have no unsupported keywords"
        );
    }

    #[test]
    fn test_check_keywords_unsupported() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "pattern": "^[a-z]+$" }
            }
        });
        let unsupported = check_schema_keywords(&schema);
        assert!(unsupported.iter().any(|(_, kw)| kw == "pattern"));
    }

    // =====================
    // Debug tests
    // =====================

    #[test]
    fn test_slot_filler_debug() {
        let provider = Arc::new(MockLlmProvider::single("{}".to_string()));
        let filler = SlotFiller::new(provider);
        let debug_str = format!("{filler:?}");
        assert!(debug_str.contains("SlotFiller"));
        assert!(debug_str.contains("mock-llm"));
    }

    #[test]
    fn test_model_size_display() {
        assert_eq!(ModelSize::Small.to_string(), "small");
        assert_eq!(ModelSize::Large.to_string(), "large");
    }

    #[test]
    fn test_schema_complexity_display() {
        assert_eq!(SchemaComplexity::Simple.to_string(), "simple");
        assert_eq!(SchemaComplexity::Complex.to_string(), "complex");
    }

    #[test]
    fn test_schema_validation_error_display() {
        let error = SchemaValidationError {
            field_path: "$root.path".to_string(),
            message: "required field is missing".to_string(),
        };
        let s = error.to_string();
        assert!(s.contains("$root.path"));
        assert!(s.contains("missing"));
    }
}
