//! A tiny stdio MCP server for tests. Tools: `echo` returns its `text`; `env_matches` says
//! whether an environment variable of this process has a given value, so a test can check that a
//! secret reached the child without the value passing through any output.

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};

struct Fake;

fn schema(properties: Value) -> Map<String, Value> {
    json!({ "type": "object", "properties": properties })
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn text(value: &Map<String, Value>, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

impl ServerHandler for Fake {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![
            Tool::new(
                "echo",
                "Returns its text",
                schema(json!({ "text": { "type": "string" } })),
            ),
            Tool::new(
                "env_matches",
                "Whether an environment variable has the given value",
                schema(json!({ "name": { "type": "string" }, "value": { "type": "string" } })),
            ),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = request.arguments.unwrap_or_default();
        let answer = match request.name.as_ref() {
            "echo" => text(&args, "text"),
            "env_matches" => {
                let actual = std::env::var(text(&args, "name")).ok();
                (actual.as_deref() == Some(text(&args, "value").as_str())).to_string()
            }
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown tool {other}"),
                    None,
                ));
            }
        };
        Ok(CallToolResponse::Complete(CallToolResult::success(vec![
            ContentBlock::text(answer),
        ])))
    }
}

#[tokio::main]
async fn main() {
    match Fake.serve(rmcp::transport::stdio()).await {
        Ok(service) => {
            let _ = service.waiting().await;
        }
        Err(e) => {
            eprintln!("fake mcp server: {e}");
            std::process::exit(1);
        }
    }
}
