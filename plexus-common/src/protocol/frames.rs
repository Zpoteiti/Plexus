//! WebSocket text frames. PROTOCOL.md §2.
//!
//! All frames serialize via serde with `#[serde(tag = "type")]` —
//! `{"type": "<name>", ...fields}` on the wire.

use crate::protocol::types::{DeviceConfig, McpSchemas};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// All WebSocket text frames, internally tagged by `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsFrame {
    Hello(HelloFrame),
    HelloAck(HelloAckFrame),
    ToolCall(ToolCallFrame),
    ToolResult(ToolResultFrame),
    RegisterMcp(RegisterMcpFrame),
    ConfigUpdate(ConfigUpdateFrame),
    TransferBegin(TransferBeginFrame),
    TransferProgress(TransferProgressFrame),
    TransferEnd(TransferEndFrame),
    Ping(PingFrame),
    Pong(PongFrame),
    Error(ErrorFrame),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloFrame {
    pub id: Uuid,
    pub version: String,
    pub client_version: String,
    pub os: String,
    pub caps: HelloCaps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloCaps {
    pub sandbox: String,
    pub exec: bool,
    pub fs: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAckFrame {
    pub id: Uuid,
    pub device_name: String,
    pub user_id: Uuid,
    pub config: DeviceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFrame {
    pub id: Uuid,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultFrame {
    pub id: Uuid,
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterMcpFrame {
    pub id: Uuid,
    #[serde(default)]
    pub mcp_servers: Vec<McpSchemas>,
    #[serde(default)]
    pub spawn_failures: Vec<SpawnFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnFailure {
    pub server_name: String,
    pub error: String,
    pub failed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigUpdateFrame {
    pub id: Uuid,
    pub config: DeviceConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    ClientToServer,
    ServerToClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferBeginFrame {
    pub id: Uuid,
    pub direction: TransferDirection,
    pub src_device: String,
    pub src_path: String,
    pub dst_device: String,
    pub dst_path: String,
    pub total_bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgressFrame {
    pub id: Uuid,
    pub bytes_sent: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferEndFrame {
    pub id: Uuid,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingFrame {
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PongFrame {
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    fn id() -> Uuid {
        Uuid::now_v7()
    }

    #[test]
    fn hello_roundtrip() {
        let frame = WsFrame::Hello(HelloFrame {
            id: id(),
            version: "1".into(),
            client_version: "0.1.0".into(),
            os: "linux".into(),
            caps: HelloCaps {
                sandbox: "bwrap".into(),
                exec: true,
                fs: "rw".into(),
            },
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"type\":\"hello\""));
        let back: WsFrame = serde_json::from_str(&json).unwrap();
        assert_matches_hello(&frame, &back);
    }

    #[test]
    fn hello_ack_roundtrip() {
        let frame = WsFrame::HelloAck(HelloAckFrame {
            id: id(),
            device_name: "mac-mini".into(),
            user_id: Uuid::now_v7(),
            config: crate::protocol::types::DeviceConfig {
                workspace_path: "/home/alice/.plexus".into(),
                fs_policy: crate::protocol::types::FsPolicy::Sandbox,
                shell_timeout_max: 300,
                ssrf_whitelist: vec![],
                mcp_servers: serde_json::json!({}),
            },
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"type\":\"hello_ack\""));
        let _back: WsFrame = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn tool_call_roundtrip() {
        let frame = WsFrame::ToolCall(ToolCallFrame {
            id: id(),
            name: "exec".into(),
            args: serde_json::json!({"command": "git status", "timeout": 60}),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"type\":\"tool_call\""));
        let back: WsFrame = serde_json::from_str(&json).unwrap();
        if let WsFrame::ToolCall(tc) = back {
            assert_eq!(tc.name, "exec");
        } else {
            panic!("expected ToolCall variant");
        }
    }

    #[test]
    fn tool_result_roundtrip_success() {
        let req_id = id();
        let frame = WsFrame::ToolResult(ToolResultFrame {
            id: req_id,
            content: "ok".into(),
            is_error: false,
            code: None,
        });
        let json = serde_json::to_string(&frame).unwrap();
        let back: WsFrame = serde_json::from_str(&json).unwrap();
        if let WsFrame::ToolResult(r) = back {
            assert_eq!(r.id, req_id);
            assert_eq!(r.content, "ok");
            assert!(!r.is_error);
        } else {
            panic!("expected ToolResult variant");
        }
    }

    #[test]
    fn tool_result_roundtrip_error_with_code() {
        let frame = WsFrame::ToolResult(ToolResultFrame {
            id: id(),
            content: "MCP server 'google' is not running".into(),
            is_error: true,
            code: Some("mcp_unavailable".into()),
        });
        let json = serde_json::to_string(&frame).unwrap();
        let back: WsFrame = serde_json::from_str(&json).unwrap();
        if let WsFrame::ToolResult(r) = back {
            assert!(r.is_error);
            assert_eq!(r.code.as_deref(), Some("mcp_unavailable"));
        } else {
            panic!();
        }
    }

    #[test]
    fn register_mcp_roundtrip_with_failures() {
        let frame = WsFrame::RegisterMcp(RegisterMcpFrame {
            id: id(),
            mcp_servers: vec![],
            spawn_failures: vec![SpawnFailure {
                server_name: "google".into(),
                error: "subprocess exited code 1; stderr: GOOGLE_API_KEY env var not set".into(),
                failed_at: "2026-04-27T12:00:00Z".into(),
            }],
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"type\":\"register_mcp\""));
        let back: WsFrame = serde_json::from_str(&json).unwrap();
        if let WsFrame::RegisterMcp(r) = back {
            assert_eq!(r.spawn_failures.len(), 1);
            assert_eq!(r.spawn_failures[0].server_name, "google");
        } else {
            panic!();
        }
    }

    #[test]
    fn config_update_roundtrip() {
        let frame = WsFrame::ConfigUpdate(ConfigUpdateFrame {
            id: id(),
            config: crate::protocol::types::DeviceConfig {
                workspace_path: "/home/alice/.plexus".into(),
                fs_policy: crate::protocol::types::FsPolicy::Unrestricted,
                shell_timeout_max: 600,
                ssrf_whitelist: vec!["10.180.20.30:8080".into()],
                mcp_servers: serde_json::json!({}),
            },
        });
        let json = serde_json::to_string(&frame).unwrap();
        let _back: WsFrame = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn transfer_begin_roundtrip() {
        let frame = WsFrame::TransferBegin(TransferBeginFrame {
            id: id(),
            direction: TransferDirection::ClientToServer,
            src_device: "mac-mini".into(),
            src_path: "/home/alice/.plexus/.attachments/photo.jpg".into(),
            dst_device: "server".into(),
            dst_path: "/alice-uuid/.attachments/photo.jpg".into(),
            total_bytes: 2_457_600,
            sha256: "5e884898da280471".into(),
            mime: Some("image/jpeg".into()),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"type\":\"transfer_begin\""));
        let _back: WsFrame = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn transfer_progress_roundtrip() {
        let frame = WsFrame::TransferProgress(TransferProgressFrame {
            id: id(),
            bytes_sent: 1_048_576,
        });
        let json = serde_json::to_string(&frame).unwrap();
        let _back: WsFrame = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn transfer_end_success_roundtrip() {
        let frame = WsFrame::TransferEnd(TransferEndFrame {
            id: id(),
            ok: true,
            error: None,
            sha256: Some("5e884898da280471".into()),
        });
        let json = serde_json::to_string(&frame).unwrap();
        let _back: WsFrame = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn transfer_end_failure_roundtrip() {
        let frame = WsFrame::TransferEnd(TransferEndFrame {
            id: id(),
            ok: false,
            error: Some("sha256_mismatch".into()),
            sha256: None,
        });
        let json = serde_json::to_string(&frame).unwrap();
        let back: WsFrame = serde_json::from_str(&json).unwrap();
        if let WsFrame::TransferEnd(e) = back {
            assert!(!e.ok);
            assert_eq!(e.error.as_deref(), Some("sha256_mismatch"));
        } else {
            panic!();
        }
    }

    #[test]
    fn ping_pong_roundtrip() {
        let p = WsFrame::Ping(PingFrame { id: id() });
        let pong = WsFrame::Pong(PongFrame { id: id() });
        let _: WsFrame = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        let _: WsFrame = serde_json::from_str(&serde_json::to_string(&pong).unwrap()).unwrap();
    }

    #[test]
    fn error_frame_roundtrip() {
        let frame = WsFrame::Error(ErrorFrame {
            id: Some(id()),
            code: "malformed_frame".into(),
            message: "expected field 'name'".into(),
        });
        let json = serde_json::to_string(&frame).unwrap();
        let _back: WsFrame = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn unknown_type_fails_deserialize() {
        let json = r#"{"type": "totally_unknown", "id": "abc"}"#;
        let result: Result<WsFrame, _> = serde_json::from_str(json);
        assert!(result.is_err(), "should fail on unknown frame type");
    }

    fn assert_matches_hello(a: &WsFrame, b: &WsFrame) {
        if let (WsFrame::Hello(a), WsFrame::Hello(b)) = (a, b) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.version, b.version);
            assert_eq!(a.client_version, b.client_version);
            assert_eq!(a.os, b.os);
        } else {
            panic!("not Hello variants");
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use uuid::Uuid;

    fn arb_uuid() -> impl Strategy<Value = Uuid> {
        any::<[u8; 16]>().prop_map(Uuid::from_bytes)
    }

    fn arb_string() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 _.,/:-]{0,40}".prop_map(String::from)
    }

    proptest! {
        #[test]
        fn ping_roundtrip(uuid in arb_uuid()) {
            let frame = WsFrame::Ping(PingFrame { id: uuid });
            let json = serde_json::to_string(&frame).unwrap();
            let back: WsFrame = serde_json::from_str(&json).unwrap();
            if let WsFrame::Ping(p) = back {
                prop_assert_eq!(p.id, uuid);
            } else {
                prop_assert!(false, "expected Ping variant");
            }
        }

        #[test]
        fn tool_call_roundtrip(
            uuid in arb_uuid(),
            name in arb_string(),
        ) {
            let frame = WsFrame::ToolCall(ToolCallFrame {
                id: uuid,
                name: name.clone(),
                args: serde_json::json!({}),
            });
            let json = serde_json::to_string(&frame).unwrap();
            let back: WsFrame = serde_json::from_str(&json).unwrap();
            if let WsFrame::ToolCall(t) = back {
                prop_assert_eq!(t.id, uuid);
                prop_assert_eq!(t.name, name);
            } else {
                prop_assert!(false);
            }
        }
    }
}
