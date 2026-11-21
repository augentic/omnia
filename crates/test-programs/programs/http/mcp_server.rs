//! A `wasi:http/handler` guest serving an MCP server through
//! `omnia_sdk::mcp::router`. Its one tool awaits a capability before it
//! answers: `greet` reads the `GREETING` setting from `omnia:config`.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use omnia_sdk::mcp::{self, CallToolResult, Implementation, McpError, McpServer, Tool};
use omnia_sdk::{config, http};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

http::export!(router);

fn router() -> Router {
    mcp::router(Greeter)
}

#[derive(Deserialize, JsonSchema)]
struct Greet {
    name: String,
}

struct Greeter;

impl McpServer for Greeter {
    fn info(&self) -> Implementation {
        Implementation::new("greeter", "1.0.0")
    }

    fn tools(&self) -> Vec<Tool> {
        vec![Tool::of::<Greet>("greet", "Greet someone by name.")]
    }

    async fn call_tool(&self, name: &str, arguments: &Value) -> Result<CallToolResult, McpError> {
        if name != "greet" {
            return Err(McpError::unknown_tool(name));
        }
        let Greet { name } = mcp::arguments(arguments)?;
        let greeting: String = config::require("GREETING")
            .await
            .map_err(|error| McpError::internal(error.to_string()))?;
        Ok(CallToolResult::text(format!("{greeting}, {name}")))
    }
}
