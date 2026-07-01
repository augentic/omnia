//! # MCP example — docs guest
//!
//! Serves a few compiled-in documents to agent backends as a stateless MCP
//! server: it implements [`omnia_guest::mcp::McpServer`] and serves
//! [`omnia_guest::mcp::router`] from its `wasi:http` handler. `omnia.toml` routes
//! `/mcp/docs` here.

#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use omnia_guest::mcp::{
    self, CallToolResult, Implementation, McpError, McpServer, Resource, ResourceContents, Tool,
};
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct DocsGuest;
wasip3::http::service::export!(DocsGuest);

impl Guest for DocsGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        omnia_wasi_http::serve(mcp::router(Arc::new(Docs)), request).await
    }
}

/// The compiled-in prose corpus as `(name, title, body)` triples.
const DOCS: &[(&str, &str, &str)] = &[
    ("overview", "Widget Service Overview", include_str!("docs/overview.md")),
    ("api-reference", "Widget Service API Reference", include_str!("docs/api-reference.md")),
    ("style-guide", "Widget Service Style Guide", include_str!("docs/style-guide.md")),
];

fn find_doc(name: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    DOCS.iter().find(|(doc_name, ..)| *doc_name == name)
}

struct Docs;

impl McpServer for Docs {
    fn info(&self) -> Implementation {
        Implementation::new("omnia-docs", env!("CARGO_PKG_VERSION"))
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new(
                "list_docs",
                "List the name and title of every available document. Call this first to \
                 discover what documentation exists before answering questions about the \
                 Widget Service.",
                json!({ "type": "object", "properties": {} }),
            ),
            Tool::new(
                "read_doc",
                "Read one document in full by its `name` (as returned by `list_docs`).",
                json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "the document name, e.g. `overview`",
                        }
                    },
                    "required": ["name"],
                }),
            ),
        ]
    }

    fn call_tool(&self, name: &str, arguments: &Value) -> Result<CallToolResult, McpError> {
        match name {
            "list_docs" => {
                let listing = DOCS
                    .iter()
                    .map(|(doc_name, title, _)| format!("- {doc_name}: {title}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(CallToolResult::text(listing))
            }
            "read_doc" => {
                let Some(doc) = arguments.get("name").and_then(Value::as_str) else {
                    return Err(McpError::invalid_params("missing `name`"));
                };
                find_doc(doc).map_or_else(
                    || Ok(CallToolResult::error(format!("no document named `{doc}`"))),
                    |(.., body)| Ok(CallToolResult::text(*body)),
                )
            }
            other => Err(McpError::method_not_found(format!("unknown tool `{other}`"))),
        }
    }

    fn resources(&self) -> Vec<Resource> {
        DOCS.iter()
            .map(|(name, title, _)| {
                Resource::new(
                    format!("doc://{name}"),
                    *title,
                    format!("The {title} document."),
                    "text/markdown",
                )
            })
            .collect()
    }

    fn read_resource(&self, uri: &str) -> Result<ResourceContents, McpError> {
        let name = uri.strip_prefix("doc://").unwrap_or(uri);
        find_doc(name).map_or_else(
            || Err(McpError::invalid_params(format!("unknown resource `{uri}`"))),
            |(.., body)| Ok(ResourceContents::text(uri, "text/markdown", *body)),
        )
    }
}
