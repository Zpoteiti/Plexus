//! JSON Schema validation for incoming `tool_call` args.
//!
//! Uses `jsonschema` 0.30. Failures return `ToolError::InvalidArgs` with
//! a human-readable message that includes every validation error found.

use crate::errors::ToolError;
use serde_json::Value;

/// Validate `args` against `schema`. On failure, returns `ToolError::InvalidArgs`
/// with all validation errors joined by `; `.
pub fn validate_args(schema: &Value, args: &Value) -> Result<(), ToolError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| ToolError::InvalidArgs(format!("invalid schema: {e}")))?;

    let errors: Vec<String> = validator.iter_errors(args).map(|e| e.to_string()).collect();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ToolError::InvalidArgs(errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{Code, ErrorCode, ToolError};
    use serde_json::json;

    fn echo_schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer" },
                "name": { "type": "string" }
            },
            "required": ["x"]
        })
    }

    #[test]
    fn valid_args_pass() {
        let result = validate_args(&echo_schema(), &json!({"x": 42}));
        assert!(result.is_ok(), "expected ok, got {:?}", result);
    }

    #[test]
    fn valid_args_with_optional_pass() {
        let result = validate_args(&echo_schema(), &json!({"x": 42, "name": "alice"}));
        assert!(result.is_ok());
    }

    #[test]
    fn missing_required_field_rejected() {
        let result = validate_args(&echo_schema(), &json!({"name": "alice"}));
        let err = result.unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidArgs);
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn wrong_type_rejected() {
        let result = validate_args(&echo_schema(), &json!({"x": "not an int"}));
        assert!(matches!(result, Err(ToolError::InvalidArgs(_))));
    }

    #[test]
    fn empty_schema_accepts_anything() {
        let schema = json!({});
        assert!(validate_args(&schema, &json!({})).is_ok());
        assert!(validate_args(&schema, &json!({"anything": [1, 2, 3]})).is_ok());
    }

    #[test]
    fn error_message_lists_all_violations() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "integer" },
                "b": { "type": "string" }
            },
            "required": ["a", "b"]
        });
        // Missing both required fields
        let result = validate_args(&schema, &json!({}));
        let err = result.unwrap_err();
        match err {
            ToolError::InvalidArgs(msg) => {
                assert!(msg.contains("a") || msg.contains("b"));
            }
            _ => panic!("expected InvalidArgs"),
        }
    }
}
