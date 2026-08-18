//! Template expression resolver for workflow step inputs.
//!
//! Resolves `${...}` template expressions in JSON values:
//!
//! - `${variables.<name>}` — workflow-level variables
//! - `${steps.<step_id>.output.<field>}` — output from a previous step
//!
//! Also supports condition expressions for conditional step execution:
//!
//! - `${steps.check.output.size} > 1000`
//! - `${steps.check.output.success} == true`
//! - `${steps.check.output.status} == "ok"`

use serde_json::Value;

use crate::types::{WorkflowError, WorkflowResult};

/// The execution context for template expression resolution.
///
/// Contains all data that can be referenced by template expressions:
/// - `variables`: workflow-level variables provided by the user
/// - `steps`: completed step outputs, keyed by step ID
#[derive(Debug, Clone, Default)]
pub struct ExpressionContext {
    /// Workflow variables, keyed by name.
    pub variables: serde_json::Map<String, Value>,

    /// Completed step outputs, keyed by step ID.
    /// Each value is the full JSON output of that step.
    pub steps: serde_json::Map<String, Value>,
}

impl ExpressionContext {
    /// Create a new empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a context from a variables map.
    pub fn with_variables(variables: serde_json::Map<String, Value>) -> Self {
        Self {
            variables,
            steps: serde_json::Map::new(),
        }
    }

    /// Add a step's output to the context.
    pub fn with_step_output(mut self, step_id: &str, output: Value) -> Self {
        self.steps.insert(step_id.to_string(), output);
        self
    }

    /// Set a step's output in the context.
    pub fn set_step_output(&mut self, step_id: &str, output: Value) {
        self.steps.insert(step_id.to_string(), output);
    }

    /// Set a variable in the context.
    pub fn set_variable(&mut self, name: &str, value: Value) {
        self.variables.insert(name.to_string(), value);
    }
}

/// Resolve all `${...}` template expressions in a JSON value.
///
/// Recursively walks the JSON tree and replaces any string containing
/// `${...}` with the resolved value.
///
/// # Errors
///
/// Returns `WorkflowError::ExpressionResolution` if a referenced path
/// does not exist.
pub fn resolve_value(value: &Value, ctx: &ExpressionContext) -> WorkflowResult<Value> {
    match value {
        Value::String(s) => {
            if s.contains("${") {
                let resolved = resolve_string(s, ctx)?;
                // If the entire string is a single expression, parse it as JSON
                // to preserve the original type (number, boolean, object, etc.)
                if is_single_expression(s) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resolved) {
                        return Ok(parsed);
                    }
                }
                Ok(Value::String(resolved))
            } else {
                Ok(value.clone())
            }
        }
        Value::Object(map) => {
            let mut result = serde_json::Map::new();
            for (k, v) in map {
                result.insert(k.clone(), resolve_value(v, ctx)?);
            }
            Ok(Value::Object(result))
        }
        Value::Array(arr) => {
            let mut result = Vec::new();
            for v in arr {
                result.push(resolve_value(v, ctx)?);
            }
            Ok(Value::Array(result))
        }
        // Non-string scalars pass through unchanged
        _ => Ok(value.clone()),
    }
}

/// Check if a string is a single `${...}` expression (nothing before or after).
fn is_single_expression(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed.starts_with("${") && trimmed.ends_with('}') && !trimmed[2..].contains("${")
}

/// Resolve all `${...}` expressions in a string.
///
/// If the string is a single expression, the resolved value is returned
/// as-is (may be a non-string type). If the string contains multiple
/// expressions or has text outside expressions, all are resolved and
/// concatenated into a string.
fn resolve_string(s: &str, ctx: &ExpressionContext) -> WorkflowResult<String> {
    let mut result = String::new();
    let mut remaining = s;

    while let Some(start) = remaining.find("${") {
        // Add text before the expression
        result.push_str(&remaining[..start]);

        // Find the closing brace
        let after_start = &remaining[start + 2..];
        let end = after_start
            .find('}')
            .ok_or_else(|| WorkflowError::ExpressionResolution {
                reason: format!("unclosed expression in: {s}"),
            })?;

        let expr = &after_start[..end];
        let resolved = resolve_expression(expr.trim(), ctx)?;

        // Append the resolved value as a string
        match &resolved {
            Value::String(s) => result.push_str(s),
            other => result.push_str(&other.to_string()),
        }

        // Move past the expression
        remaining = &after_start[end + 1..];
    }

    // Append any remaining text
    result.push_str(remaining);
    Ok(result)
}

/// Resolve a single expression path like `variables.input_path`
/// or `steps.read.output.content`.
fn resolve_expression(expr: &str, ctx: &ExpressionContext) -> WorkflowResult<Value> {
    let parts: Vec<&str> = expr.split('.').collect();
    if parts.is_empty() {
        return Err(WorkflowError::ExpressionResolution {
            reason: format!("empty expression: {expr}"),
        });
    }

    match parts[0] {
        "variables" => {
            if parts.len() < 2 {
                return Err(WorkflowError::ExpressionResolution {
                    reason: "variables expression requires a name: ${variables.<name>}".to_string(),
                });
            }
            let name = parts[1];
            ctx.variables
                .get(name)
                .cloned()
                .ok_or_else(|| WorkflowError::ExpressionResolution {
                    reason: format!("variable not found: {name}"),
                })
                .and_then(|v| navigate_path(&v, &parts[2..]))
        }
        "steps" => {
            if parts.len() < 3 {
                return Err(WorkflowError::ExpressionResolution {
                    reason: "steps expression requires id and field: ${steps.<id>.<field>}"
                        .to_string(),
                });
            }
            let step_id = parts[1];
            let step_output = ctx.steps.get(step_id).cloned().ok_or_else(|| {
                WorkflowError::ExpressionResolution {
                    reason: format!("step output not found: {step_id}"),
                }
            })?;
            // `${steps.<id>.output.<field>}` and `${steps.<id>.<field>}` are
            // equivalent: `output` is a view alias for the step's raw output
            // object. This keeps the engine's storage (raw output) consistent
            // with the workflow spec's `${task_id.output}` reference form.
            if parts[2] == "output" {
                navigate_path(&step_output, &parts[3..])
            } else {
                navigate_path(&step_output, &parts[2..])
            }
        }
        // Bare root: fall back to a workflow variable of that name, so an
        // `iterate` binding (`as_var: item`) is referenced as `${item.field}`
        // as well as `${variables.item.field}` (P18). Only names that actually
        // exist as variables resolve — anything else is still an error.
        other if ctx.variables.contains_key(other) => {
            let v = ctx.variables[other].clone();
            navigate_path(&v, &parts[1..])
        }
        _ => Err(WorkflowError::ExpressionResolution {
            reason: format!(
                "unknown expression root: {} (expected 'variables', 'steps', or a variable name)",
                parts[0]
            ),
        }),
    }
}

/// Navigate a JSON path from a starting value.
///
/// For example, given `{"output": {"content": "hello"}}` and path `["output", "content"]`,
/// returns `Value::String("hello")`.
fn navigate_path(value: &Value, path: &[&str]) -> WorkflowResult<Value> {
    let mut current = value;
    for key in path {
        current = current
            .get(key)
            .ok_or_else(|| WorkflowError::ExpressionResolution {
                reason: format!("path key not found: {key}"),
            })?;
    }
    Ok(current.clone())
}

/// Evaluate a condition expression to a boolean.
///
/// Supported formats:
/// - `${path} > <number>`
/// - `${path} < <number>`
/// - `${path} >= <number>`
/// - `${path} <= <number>`
/// - `${path} == <value>` (number, true, false, "string")
/// - `${path} != <value>`
/// - Bare `${path}` — truthy check (non-null, non-false, non-empty)
pub fn evaluate_condition(condition: &str, ctx: &ExpressionContext) -> WorkflowResult<bool> {
    let condition = condition.trim();

    // Find the comparison operator (if any)
    // Order matters: check two-char operators first
    for op in [">=", "<=", "==", "!="] {
        if let Some(pos) = condition.find(op) {
            let left = condition[..pos].trim();
            let right = condition[pos + op.len()..].trim();
            let left_val = resolve_expression_str(left, ctx)?;
            let right_val = parse_literal(right)?;
            return Ok(compare_values(&left_val, op, &right_val));
        }
    }

    // Single-char operators (check after two-char to avoid false matches)
    for op in ['>', '<'] {
        if let Some(pos) = condition.find(op) {
            // Make sure it's not part of >= or <=
            let before = &condition[..pos];
            let after_char = condition[pos + 1..].chars().next();
            if after_char == Some('=') {
                continue; // Already handled by >= or <=
            }
            let left = before.trim();
            let right = condition[pos + 1..].trim();
            let left_val = resolve_expression_str(left, ctx)?;
            let right_val = parse_literal(right)?;
            return Ok(compare_values(&left_val, &op.to_string(), &right_val));
        }
    }

    // No operator — truthy check on the bare expression
    let val = resolve_expression_str(condition, ctx)?;
    Ok(is_truthy(&val))
}

/// Resolve an expression string that may be a `${...}` template or a literal.
fn resolve_expression_str(s: &str, ctx: &ExpressionContext) -> WorkflowResult<Value> {
    let s = s.trim();
    if s.starts_with("${") && s.ends_with('}') {
        let inner = &s[2..s.len() - 1];
        resolve_expression(inner.trim(), ctx)
    } else {
        parse_literal(s)
    }
}

/// Parse a literal value from a string.
///
/// Recognizes: numbers, true, false, null, and quoted strings.
fn parse_literal(s: &str) -> WorkflowResult<Value> {
    let s = s.trim();

    // Quoted string
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return Ok(Value::String(s[1..s.len() - 1].to_string()));
    }

    // Boolean
    if s == "true" {
        return Ok(Value::Bool(true));
    }
    if s == "false" {
        return Ok(Value::Bool(false));
    }

    // Null
    if s == "null" {
        return Ok(Value::Null);
    }

    // Number (integer or float)
    if let Ok(n) = s.parse::<i64>() {
        return Ok(Value::Number(serde_json::Number::from(n)));
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(f) {
            return Ok(Value::Number(num));
        }
    }

    // Fallback: treat as string
    Ok(Value::String(s.to_string()))
}

/// Compare two JSON values using a comparison operator.
fn compare_values(left: &Value, op: &str, right: &Value) -> bool {
    match op {
        "==" => values_equal(left, right),
        "!=" => !values_equal(left, right),
        ">" | "<" | ">=" | "<=" => compare_ordered(left, op, right),
        _ => false,
    }
}

/// Check equality of two JSON values.
fn values_equal(left: &Value, right: &Value) -> bool {
    // Handle number comparison (int vs float)
    if let (Some(l), Some(r)) = (left.as_f64(), right.as_f64()) {
        return (l - r).abs() < f64::EPSILON;
    }
    left == right
}

/// Compare two values with ordering operators (>, <, >=, <=).
fn compare_ordered(left: &Value, op: &str, right: &Value) -> bool {
    // Try numeric comparison first
    if let (Some(l), Some(r)) = (left.as_f64(), right.as_f64()) {
        return match op {
            ">" => l > r,
            "<" => l < r,
            ">=" => l >= r,
            "<=" => l <= r,
            _ => false,
        };
    }

    // Fall back to string comparison
    if let (Some(l), Some(r)) = (left.as_str(), right.as_str()) {
        return match op {
            ">" => l > r,
            "<" => l < r,
            ">=" => l >= r,
            "<=" => l <= r,
            _ => false,
        };
    }

    false
}

/// Check if a JSON value is truthy.
///
/// - `null` → false
/// - `false` → false
/// - `0` → false
/// - `""` → false
/// - everything else → true
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_context() -> ExpressionContext {
        let mut ctx = ExpressionContext::new();
        ctx.set_variable("input_path", json!("/tmp/doc.txt"));
        ctx.set_variable("max_length", json!(200));
        ctx.set_variable("flag", json!(true));
        ctx.set_step_output(
            "read",
            json!({
                "content": "Hello world",
                "size": 11,
                "encoding": "utf-8"
            }),
        );
        ctx.set_step_output(
            "check",
            json!({
                "success": true,
                "status": "ok",
                "count": 42
            }),
        );
        ctx
    }

    // --- resolve_value tests ---

    #[test]
    fn test_resolve_variable_string() {
        let ctx = make_context();
        let input = json!({"path": "${variables.input_path}"});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["path"], "/tmp/doc.txt");
    }

    #[test]
    fn test_resolve_variable_number() {
        let ctx = make_context();
        let input = json!({"max_length": "${variables.max_length}"});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["max_length"], 200);
    }

    #[test]
    fn test_resolve_step_output_nested() {
        let ctx = make_context();
        let input = json!({"text": "${steps.read.output.content}"});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["text"], "Hello world");
    }

    #[test]
    fn test_resolve_step_output_direct() {
        let ctx = make_context();
        // When the step output is accessed directly (no .output prefix),
        // we navigate into the stored JSON
        let input = json!({"size": "${steps.read.size}"});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["size"], 11);
    }

    #[test]
    fn test_resolve_mixed_string() {
        let ctx = make_context();
        let input = json!({"path": "${variables.input_path}.bak"});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["path"], "/tmp/doc.txt.bak");
    }

    #[test]
    fn test_resolve_multiple_expressions() {
        let ctx = make_context();
        let input = json!({"msg": "Read ${steps.read.size} bytes from ${variables.input_path}"});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["msg"], "Read 11 bytes from /tmp/doc.txt");
    }

    #[test]
    fn test_resolve_no_expressions() {
        let ctx = make_context();
        let input = json!({"key": "plain string", "num": 42});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_resolve_nested_object() {
        let ctx = make_context();
        let input = json!({
            "config": {
                "source": "${variables.input_path}",
                "limit": "${variables.max_length}"
            }
        });
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["config"]["source"], "/tmp/doc.txt");
        assert_eq!(result["config"]["limit"], 200);
    }

    #[test]
    fn test_resolve_array() {
        let ctx = make_context();
        let input = json!({
            "paths": ["${variables.input_path}", "/tmp/other.txt"]
        });
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["paths"][0], "/tmp/doc.txt");
        assert_eq!(result["paths"][1], "/tmp/other.txt");
    }

    #[test]
    fn test_resolve_boolean_variable() {
        let ctx = make_context();
        let input = json!({"flag": "${variables.flag}"});
        let result = resolve_value(&input, &ctx).unwrap();
        assert_eq!(result["flag"], true);
    }

    // --- Error cases ---

    #[test]
    fn test_resolve_unknown_variable() {
        let ctx = make_context();
        let input = json!({"x": "${variables.nonexistent}"});
        let result = resolve_value(&input, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_unknown_step() {
        let ctx = make_context();
        let input = json!({"x": "${steps.unknown.output.content}"});
        let result = resolve_value(&input, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_unknown_path() {
        let ctx = make_context();
        let input = json!({"x": "${steps.read.output.nonexistent}"});
        let result = resolve_value(&input, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_unclosed_expression() {
        let ctx = make_context();
        let input = json!({"x": "${variables.input_path"});
        let result = resolve_value(&input, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_unknown_root() {
        let ctx = make_context();
        let input = json!({"x": "${config.path}"});
        let result = resolve_value(&input, &ctx);
        assert!(result.is_err());
    }

    // --- Condition evaluation tests ---

    #[test]
    fn test_condition_greater_than() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.read.size} > 5", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.read.size} > 20", &ctx).unwrap());
    }

    #[test]
    fn test_condition_less_than() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.read.size} < 20", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.read.size} < 5", &ctx).unwrap());
    }

    #[test]
    fn test_condition_greater_equal() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.read.size} >= 11", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.read.size} >= 12", &ctx).unwrap());
    }

    #[test]
    fn test_condition_less_equal() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.read.size} <= 11", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.read.size} <= 10", &ctx).unwrap());
    }

    #[test]
    fn test_condition_equal_number() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.read.size} == 11", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.read.size} == 10", &ctx).unwrap());
    }

    #[test]
    fn test_condition_equal_string() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.read.encoding} == \"utf-8\"", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.read.encoding} == \"ascii\"", &ctx).unwrap());
    }

    #[test]
    fn test_condition_equal_boolean() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.check.success} == true", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.check.success} == false", &ctx).unwrap());
    }

    #[test]
    fn test_condition_not_equal() {
        let ctx = make_context();
        assert!(evaluate_condition("${steps.read.size} != 20", &ctx).unwrap());
        assert!(!evaluate_condition("${steps.read.size} != 11", &ctx).unwrap());
    }

    #[test]
    fn test_condition_truthy_bare() {
        let ctx = make_context();
        assert!(evaluate_condition("${variables.flag}", &ctx).unwrap());
        assert!(evaluate_condition("${variables.missing}", &ctx).is_err());
    }

    #[test]
    fn test_condition_truthy_string() {
        let mut ctx = ExpressionContext::new();
        ctx.set_variable("nonempty", json!("hello"));
        ctx.set_variable("empty", json!(""));
        assert!(evaluate_condition("${variables.nonempty}", &ctx).unwrap());
        assert!(!evaluate_condition("${variables.empty}", &ctx).unwrap());
    }

    // --- is_single_expression tests ---

    #[test]
    fn test_single_expression() {
        assert!(is_single_expression("${variables.x}"));
        assert!(is_single_expression("  ${variables.x}  "));
    }

    #[test]
    fn test_not_single_expression() {
        assert!(!is_single_expression("prefix${variables.x}"));
        assert!(!is_single_expression("${variables.x}suffix"));
        assert!(!is_single_expression("${variables.x}${variables.y}"));
        assert!(!is_single_expression("plain string"));
    }

    // --- parse_literal tests ---

    #[test]
    fn test_parse_literal_int() {
        assert_eq!(parse_literal("42").unwrap(), json!(42));
    }

    #[test]
    fn test_parse_literal_float() {
        assert_eq!(parse_literal("2.5").unwrap(), json!(2.5));
    }

    #[test]
    fn test_parse_literal_bool() {
        assert_eq!(parse_literal("true").unwrap(), json!(true));
        assert_eq!(parse_literal("false").unwrap(), json!(false));
    }

    #[test]
    fn test_parse_literal_null() {
        assert_eq!(parse_literal("null").unwrap(), Value::Null);
    }

    #[test]
    fn test_parse_literal_quoted_string() {
        assert_eq!(parse_literal("\"hello\"").unwrap(), json!("hello"));
        assert_eq!(parse_literal("'world'").unwrap(), json!("world"));
    }

    // --- is_truthy tests ---

    #[test]
    fn test_is_truthy() {
        assert!(!is_truthy(&Value::Null));
        assert!(!is_truthy(&json!(false)));
        assert!(!is_truthy(&json!(0)));
        assert!(!is_truthy(&json!("")));
        assert!(!is_truthy(&json!([])));
        assert!(!is_truthy(&json!({})));

        assert!(is_truthy(&json!(true)));
        assert!(is_truthy(&json!(1)));
        assert!(is_truthy(&json!("hello")));
        assert!(is_truthy(&json!([1])));
        assert!(is_truthy(&json!({"a": 1})));
    }
}
