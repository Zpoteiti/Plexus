//! Integration test stitching schemas → validation → result wrap → frame ser/de.
//!
//! Exercises only the public API of `plexus-common`. Catches breaks at
//! module boundaries that unit tests would miss.

use plexus_common::consts::UNTRUSTED_TOOL_RESULT_PREFIX;
use plexus_common::errors::ToolError;
use plexus_common::protocol::{ToolResultFrame, WsFrame};
use plexus_common::tools::result::wrap_result;
use plexus_common::tools::schemas::READ_FILE_SCHEMA;
use plexus_common::tools::validate::validate_args;
use serde_json::json;
use uuid::Uuid;

#[test]
fn read_file_schema_validates_and_round_trips_through_frame() {
    // 1. Validate args against the schema (schema is a tool-level wrapper;
    //    validate_args expects the inner input_schema for the actual JSON
    //    Schema validation step).
    let input_schema = &READ_FILE_SCHEMA["input_schema"];
    let valid_args = json!({"path": "MEMORY.md"});
    validate_args(input_schema, &valid_args).expect("valid args should pass");

    // 2. Reject invalid args.
    let invalid_args = json!({"offset": -5}); // missing required "path", and offset < 1
    let err = validate_args(input_schema, &invalid_args).unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));

    // 3. Build a tool_result with wrapped content.
    let raw = "1|hello\n2|world";
    let wrapped = wrap_result(raw);
    assert!(wrapped.starts_with(UNTRUSTED_TOOL_RESULT_PREFIX));

    // 4. Pack into a ToolResultFrame and roundtrip via JSON.
    let frame = WsFrame::ToolResult(ToolResultFrame {
        id: Uuid::now_v7(),
        content: wrapped.clone(),
        is_error: false,
        code: None,
    });
    let json_str = serde_json::to_string(&frame).unwrap();
    assert!(json_str.contains("\"type\":\"tool_result\""));

    let back: WsFrame = serde_json::from_str(&json_str).unwrap();
    if let WsFrame::ToolResult(tr) = back {
        assert_eq!(tr.content, wrapped);
        assert!(!tr.is_error);
    } else {
        panic!("expected ToolResult variant after roundtrip");
    }
}

#[test]
fn error_path_carries_typed_code() {
    use plexus_common::errors::ErrorCode;

    let frame = WsFrame::ToolResult(ToolResultFrame {
        id: Uuid::now_v7(),
        content: wrap_result("operation timed out"),
        is_error: true,
        code: Some(ErrorCode::ExecTimeout),
    });
    let json_str = serde_json::to_string(&frame).unwrap();
    // Wire format is snake_case string per ADR-046
    assert!(json_str.contains("\"code\":\"exec_timeout\""));
    let back: WsFrame = serde_json::from_str(&json_str).unwrap();
    if let WsFrame::ToolResult(tr) = back {
        assert!(tr.is_error);
        assert_eq!(tr.code, Some(ErrorCode::ExecTimeout));
    } else {
        panic!();
    }
}
