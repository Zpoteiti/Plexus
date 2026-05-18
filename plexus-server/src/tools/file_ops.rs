use crate::workspace::WorkspaceFs;
use plexus_common::{Code, ToolError, WorkspaceError};
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn call_file_tool(
    fs: &WorkspaceFs,
    user_id: Uuid,
    name: &str,
    args: Value,
) -> Result<String, ToolError> {
    let device = string_arg(&args, "plexus_device")?;
    if device != "server" {
        return Err(ToolError::InvalidArgs(
            "M1d only supports plexus_device=server".to_string(),
        ));
    }

    match name {
        "read_file" => {
            let path = string_arg(&args, "path")?;
            let bytes = fs
                .read_file(user_id, path)
                .await
                .map_err(workspace_to_tool)?;
            Ok(String::from_utf8_lossy(&bytes).to_string())
        }
        "write_file" => {
            let path = string_arg(&args, "path")?;
            let content = string_arg(&args, "content")?;
            fs.write_file(user_id, path, content.as_bytes().to_vec())
                .await
                .map_err(workspace_to_tool)?;
            Ok("written".to_string())
        }
        "edit_file" => {
            let path = string_arg(&args, "path")?;
            let old_text = string_arg(&args, "old_text")?;
            if old_text.is_empty() {
                return Err(ToolError::InvalidArgs(
                    "old_text must not be empty".to_string(),
                ));
            }
            let new_text = string_arg(&args, "new_text")?;
            let replace_all = args
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let replacements = fs
                .edit_file(user_id, path, old_text, new_text, replace_all)
                .await
                .map_err(workspace_to_tool)?;
            Ok(json!({ "replacements": replacements }).to_string())
        }
        "delete_file" => {
            let path = string_arg(&args, "path")?;
            fs.delete_file(user_id, path)
                .await
                .map_err(workspace_to_tool)?;
            Ok("deleted".to_string())
        }
        "delete_folder" => {
            let path = string_arg(&args, "path")?;
            fs.delete_folder(user_id, path)
                .await
                .map_err(workspace_to_tool)?;
            Ok("deleted".to_string())
        }
        "list_dir" => {
            let path = string_arg(&args, "path")?;
            let entries = fs
                .list_dir(user_id, path)
                .await
                .map_err(workspace_to_tool)?;
            serde_json::to_string(&entries).map_err(|err| ToolError::InvalidArgs(err.to_string()))
        }
        "glob" => {
            let pattern = string_arg(&args, "pattern")?;
            let matches = fs.glob(user_id, pattern).await.map_err(workspace_to_tool)?;
            serde_json::to_string(&matches).map_err(|err| ToolError::InvalidArgs(err.to_string()))
        }
        "grep" => {
            let pattern = string_arg(&args, "pattern")?;
            let path = args.get("path").and_then(Value::as_str);
            let matches = fs
                .grep(user_id, pattern, path)
                .await
                .map_err(workspace_to_tool)?;
            serde_json::to_string(&matches).map_err(|err| ToolError::InvalidArgs(err.to_string()))
        }
        _ => Err(ToolError::InvalidArgs(format!("unknown file tool: {name}"))),
    }
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} is required")))
}

fn workspace_to_tool(err: WorkspaceError) -> ToolError {
    ToolError::InvalidArgs(format!("{:?}: {}", err.code(), err))
}
