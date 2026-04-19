use crate::config::ClientConfig;
use crate::tools::helpers::{sanitize_path, tool_error};
use crate::tools::{Tool, ToolResult};
use plexus_common::fuzzy_match;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub struct EditFileTool;

impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn execute(
        &self,
        args: Value,
        config: &ClientConfig,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move { exec(args, &config).await })
    }
}

async fn exec(args: Value, config: &ClientConfig) -> ToolResult {
    // Accept both "path" (server schema) and legacy "file_path".
    let p = match args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(Value::as_str)
    {
        Some(p) => p,
        None => return ToolResult::error(tool_error("missing: path")),
    };
    let old = match args.get("old_text").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error(tool_error("old_text must be non-empty")),
    };
    let new = match args.get("new_text").and_then(Value::as_str) {
        Some(s) => s,
        None => return ToolResult::error(tool_error("missing: new_text")),
    };

    let path = match sanitize_path(p, config, true) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(e),
    };
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => return ToolResult::error(tool_error(&format!("read: {e}"))),
    };

    match fuzzy_match::find_match(&content, old) {
        Err(failure) => {
            let hint = if failure.hints.is_empty() {
                format!(" (best similarity: {:.0}%)", failure.best_ratio * 100.0)
            } else {
                format!(" ({})", failure.hints.join(", "))
            };
            ToolResult::error(tool_error(&format!("old_text not found in {p}{hint}")))
        }
        Ok(m) if m.count > 1 => ToolResult::error(tool_error(&format!(
            "{} matches in {p} — must be exactly 1",
            m.count
        ))),
        Ok(m) => {
            let new_content = content.replacen(&m.matched_text, new, 1);
            match tokio::fs::write(&path, &new_content).await {
                Ok(()) => ToolResult::success(format!("Edited {p}")),
                Err(e) => ToolResult::error(tool_error(&format!("write: {e}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plexus_common::protocol::FsPolicy;

    fn cfg(d: &std::path::Path) -> ClientConfig {
        ClientConfig {
            workspace: d.to_path_buf(),
            fs_policy: FsPolicy::Unrestricted,
            shell_timeout_max: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }
    }

    #[tokio::test]
    async fn test_edit_exact() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.rs");
        std::fs::write(&f, "fn main() { old() }").unwrap();
        let r = exec(
            serde_json::json!({"path": f.to_str().unwrap(), "old_text": "old()", "new_text": "new()"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 0);
        assert!(std::fs::read_to_string(&f).unwrap().contains("new()"));
    }

    #[tokio::test]
    async fn test_edit_legacy_file_path() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.rs");
        std::fs::write(&f, "fn main() { old() }").unwrap();
        let r = exec(
            serde_json::json!({"file_path": f.to_str().unwrap(), "old_text": "old()", "new_text": "new()"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test]
    async fn test_edit_no_match() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.txt");
        std::fs::write(&f, "hello").unwrap();
        let r = exec(
            serde_json::json!({"path": f.to_str().unwrap(), "old_text": "bye", "new_text": "x"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test]
    async fn test_edit_multi_match() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.txt");
        std::fs::write(&f, "aa bb aa").unwrap();
        let r = exec(
            serde_json::json!({"path": f.to_str().unwrap(), "old_text": "aa", "new_text": "cc"}),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test]
    async fn test_edit_fuzzy_indent() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.rs");
        std::fs::write(&f, "    fn f() {\n        x();\n    }").unwrap();
        let r = exec(
            serde_json::json!({
                "path": f.to_str().unwrap(),
                "old_text": "fn f() {\n    x();\n}",
                "new_text": "fn f() { y(); }"
            }),
            &cfg(d.path()),
        )
        .await;
        assert_eq!(r.exit_code, 0);
    }
}
