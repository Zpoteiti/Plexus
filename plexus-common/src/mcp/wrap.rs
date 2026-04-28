//! URI template parsing per ADR-099 — surfaces `{var}` placeholders as
//! `input_schema` properties + substitutes at call time.
//!
//! Simple `{var}` syntax only (regex `\{(\w+)\}`). RFC 6570 features
//! (operators, query strings, fragments) are NOT supported; if a real
//! MCP needs them we revisit.

use crate::errors::McpError;
use serde_json::{Value, json};

/// Extract the unique placeholder variable names from a URI template.
///
/// Order matches first-occurrence order in the template; duplicates are
/// dropped so the same `{x}` appearing twice yields one schema property
/// and one substitution pass.
pub fn parse_uri_placeholders(uri: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = uri.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        // Per ADR-099, placeholder names match `\w+` — ASCII letters, digits, underscore.
        while end < bytes.len() && bytes[end] != b'}' {
            let c = bytes[end];
            if !(c.is_ascii_alphanumeric() || c == b'_') {
                break;
            }
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'}' && end > start {
            let name = &uri[start..end];
            if !out.iter().any(|existing| existing == name) {
                out.push(name.to_string());
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Build the `input_schema` body for a resource URI template.
///
/// Static URIs (no placeholders) produce `{type:object, properties:{}, required:[]}`.
/// Templates produce one required `string` property per placeholder.
pub fn build_resource_input_schema(uri: &str) -> Value {
    let placeholders = parse_uri_placeholders(uri);
    let mut properties = serde_json::Map::new();
    for name in &placeholders {
        properties.insert(
            name.clone(),
            json!({
                "type": "string",
                "description": format!("URI template variable: {name}")
            }),
        );
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": placeholders,
    })
}

/// Substitute placeholder values into the URI template using `args`.
///
/// Each `{name}` is replaced with the string form of `args[name]`. Numeric
/// values are rendered without quotes; strings without quotes; booleans as
/// "true"/"false". Returns `McpError::CallFailed` if any placeholder has
/// no corresponding key in `args`.
pub fn substitute_uri(uri: &str, args: &Value) -> Result<String, McpError> {
    let placeholders = parse_uri_placeholders(uri);
    let mut out = uri.to_string();
    for name in placeholders {
        let value = args.get(&name).ok_or_else(|| McpError::CallFailed {
            server: "uri-substitute".to_string(),
            detail: format!("missing placeholder '{name}' in args"),
        })?;
        let replacement = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            other => other.to_string(),
        };
        out = out.replace(&format!("{{{name}}}"), &replacement);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_static_uri_no_placeholders() {
        let placeholders = parse_uri_placeholders("notion://workspace/index");
        assert!(placeholders.is_empty());
    }

    #[test]
    fn parse_single_placeholder() {
        let placeholders = parse_uri_placeholders("notion://page/{page_id}");
        assert_eq!(placeholders, vec!["page_id"]);
    }

    #[test]
    fn parse_multiple_placeholders() {
        let placeholders = parse_uri_placeholders("api://{org}/{project}/{file}");
        assert_eq!(placeholders, vec!["org", "project", "file"]);
    }

    #[test]
    fn parse_placeholder_with_underscore() {
        let placeholders = parse_uri_placeholders("api://{user_id}/profile");
        assert_eq!(placeholders, vec!["user_id"]);
    }

    #[test]
    fn build_input_schema_static() {
        let schema = build_resource_input_schema("notion://workspace/index");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].as_object().unwrap().is_empty());
        assert!(schema["required"].as_array().unwrap().is_empty());
    }

    #[test]
    fn build_input_schema_with_placeholder() {
        let schema = build_resource_input_schema("notion://page/{page_id}");
        assert_eq!(schema["type"], "object");
        let properties = schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("page_id"));
        assert_eq!(properties["page_id"]["type"], "string");
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(required, vec!["page_id"]);
    }

    #[test]
    fn substitute_static_uri_unchanged() {
        let result = substitute_uri("notion://workspace/index", &json!({})).unwrap();
        assert_eq!(result, "notion://workspace/index");
    }

    #[test]
    fn substitute_single_placeholder() {
        let result =
            substitute_uri("notion://page/{page_id}", &json!({"page_id": "abc123"})).unwrap();
        assert_eq!(result, "notion://page/abc123");
    }

    #[test]
    fn substitute_multiple_placeholders() {
        let result = substitute_uri(
            "api://{org}/{project}",
            &json!({"org": "plexus", "project": "rebuild"}),
        )
        .unwrap();
        assert_eq!(result, "api://plexus/rebuild");
    }

    #[test]
    fn substitute_missing_placeholder_returns_error() {
        let result = substitute_uri("notion://page/{page_id}", &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn substitute_extra_args_ignored() {
        // Args have an extra key that's not in the URI; substitution succeeds.
        let result = substitute_uri(
            "notion://page/{page_id}",
            &json!({"page_id": "x", "extra": "ignored"}),
        )
        .unwrap();
        assert_eq!(result, "notion://page/x");
    }

    #[test]
    fn substitute_non_string_arg_uses_string_repr() {
        // Numeric arg — substituted as its JSON-string form (without quotes).
        let result = substitute_uri("api://item/{id}", &json!({"id": 42})).unwrap();
        assert_eq!(result, "api://item/42");
    }

    #[test]
    fn duplicate_placeholders_deduplicated() {
        // `{x}` appearing twice yields one entry; schema lists it once.
        let placeholders = parse_uri_placeholders("api://{x}/{x}");
        assert_eq!(placeholders, vec!["x"]);

        let schema = build_resource_input_schema("api://{x}/{x}");
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(required, vec!["x"]);

        let result = substitute_uri("api://{x}/{x}", &json!({"x": "a"})).unwrap();
        assert_eq!(result, "api://a/a");
    }
}
