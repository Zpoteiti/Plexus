//! Test fixture: minimal MCP server speaking on stdio.
//!
//! Used by `tests/mcp_lifecycle.rs` to exercise plexus-common's MCP
//! client wrapper end-to-end without depending on real MCP servers.
//!
//! Capabilities exposed:
//! - Tool `echo` — returns the received args as text.
//! - Resource `fake://fixed` — returns "fixed-resource-content".
//! - Prompt `greet` — returns a single user message "hello from greet".

use std::future::Future;
use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        AnnotateAble, CallToolRequestParams, CallToolResult, Content, GetPromptRequestParams,
        GetPromptResult, ListPromptsResult, ListResourcesResult, ListToolsResult,
        PaginatedRequestParams, Prompt, PromptMessage, PromptMessageContent, PromptMessageRole,
        RawResource, ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{MaybeSendFuture, RequestContext},
};
use serde_json::json;

#[derive(Clone)]
struct FakeMcp;

impl ServerHandler for FakeMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        let schema = Arc::new(
            json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer" }
                }
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        std::future::ready(Ok(ListToolsResult::with_all_items(vec![Tool::new(
            "echo",
            "Echo args back as text",
            schema,
        )])))
    }

    fn call_tool(
        &self,
        params: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let args_text = serde_json::to_string(&params.arguments).unwrap_or_default();
        std::future::ready(Ok(CallToolResult::success(vec![Content::text(format!(
            "echoed: {args_text}"
        ))])))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + MaybeSendFuture + '_ {
        let resource: Resource = RawResource::new("fake://fixed", "fixed").no_annotation();
        std::future::ready(Ok(ListResourcesResult::with_all_items(vec![resource])))
    }

    fn read_resource(
        &self,
        params: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: params.uri,
                mime_type: Some("text/plain".into()),
                text: "fixed-resource-content".into(),
                meta: None,
            },
        ])))
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListPromptsResult::with_all_items(vec![Prompt::new(
            "greet",
            Some("Returns a one-line user message"),
            None,
        )])))
    }

    fn get_prompt(
        &self,
        _params: GetPromptRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(GetPromptResult::new(vec![PromptMessage::new(
            PromptMessageRole::User,
            PromptMessageContent::text("hello from greet"),
        )])))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let transport = rmcp::transport::io::stdio();
    let service = FakeMcp.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
