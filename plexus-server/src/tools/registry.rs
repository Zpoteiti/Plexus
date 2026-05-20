use crate::{tools::file_ops, workspace::WorkspaceFs};
use plexus_common::{
    ToolError,
    tools::schemas::{
        DELETE_FILE_SCHEMA, DELETE_FOLDER_SCHEMA, EDIT_FILE_SCHEMA, GLOB_SCHEMA, GREP_SCHEMA,
        LIST_DIR_SCHEMA, READ_FILE_SCHEMA, WRITE_FILE_SCHEMA,
    },
};
use serde_json::{Value, json};
use uuid::Uuid;

pub const SERVER_DEVICE: &str = "server";

pub fn merged_file_tool_schemas() -> Vec<Value> {
    [
        &*READ_FILE_SCHEMA,
        &*WRITE_FILE_SCHEMA,
        &*EDIT_FILE_SCHEMA,
        &*DELETE_FILE_SCHEMA,
        &*DELETE_FOLDER_SCHEMA,
        &*LIST_DIR_SCHEMA,
        &*GLOB_SCHEMA,
        &*GREP_SCHEMA,
    ]
    .into_iter()
    .map(inject_server_device)
    .collect()
}

#[derive(Clone)]
pub struct FileToolRegistry {
    fs: WorkspaceFs,
}

impl FileToolRegistry {
    pub fn new(fs: WorkspaceFs) -> Self {
        Self { fs }
    }

    pub async fn call(&self, user_id: Uuid, name: &str, args: Value) -> Result<String, ToolError> {
        file_ops::call_file_tool(&self.fs, user_id, name, args).await
    }
}

fn inject_server_device(schema: &Value) -> Value {
    let mut schema = schema.clone();
    let input = schema
        .get_mut("input_schema")
        .and_then(Value::as_object_mut)
        .expect("tool schema has object input_schema");
    let properties = input
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("tool schema properties is an object");
    properties.insert(
        "plexus_device".to_string(),
        json!({
            "type": "string",
            "enum": [SERVER_DEVICE],
            "description": "Which Plexus install site to execute on. M1d supports only server.",
            "x-plexus-device": true
        }),
    );

    let required = input.entry("required").or_insert_with(|| json!([]));
    let required = required
        .as_array_mut()
        .expect("tool schema required is an array");
    if !required.iter().any(|value| value == "plexus_device") {
        required.push(json!("plexus_device"));
    }

    schema
}
