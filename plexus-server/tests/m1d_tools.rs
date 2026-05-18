use plexus_common::tools::schemas::{
    DELETE_FILE_SCHEMA, DELETE_FOLDER_SCHEMA, EDIT_FILE_SCHEMA, GLOB_SCHEMA, GREP_SCHEMA,
    LIST_DIR_SCHEMA, READ_FILE_SCHEMA, WRITE_FILE_SCHEMA,
};
use plexus_server::tools::registry::{SERVER_DEVICE, merged_file_tool_schemas};
use serde_json::json;

#[test]
fn shared_file_source_schemas_do_not_contain_plexus_device() {
    for schema in [
        &*READ_FILE_SCHEMA,
        &*WRITE_FILE_SCHEMA,
        &*EDIT_FILE_SCHEMA,
        &*DELETE_FILE_SCHEMA,
        &*DELETE_FOLDER_SCHEMA,
        &*LIST_DIR_SCHEMA,
        &*GLOB_SCHEMA,
        &*GREP_SCHEMA,
    ] {
        let props = schema["input_schema"]["properties"].as_object().unwrap();
        assert!(!props.contains_key("plexus_device"));
    }
}

#[test]
fn merge_v0_injects_required_server_device() {
    let schemas = merged_file_tool_schemas();
    let read_file = schemas
        .iter()
        .find(|schema| schema["name"] == "read_file")
        .unwrap();
    let input = &read_file["input_schema"];

    assert_eq!(
        input["properties"]["plexus_device"]["enum"],
        json!([SERVER_DEVICE])
    );
    assert_eq!(
        input["properties"]["plexus_device"]["x-plexus-device"],
        json!(true)
    );
    assert!(
        input["required"]
            .as_array()
            .unwrap()
            .contains(&json!("plexus_device"))
    );
}
