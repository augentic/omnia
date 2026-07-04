//! # MCP example — docs guest
//!
//! Serves a few compiled-in documents to agent backends as a stateless MCP
//! server: it implements [`omnia_guest::mcp::McpServer`] and calls
//! [`omnia_guest::mcp::serve`] from its `wasi:http` handler. `omnia.toml` routes
//! `/mcp/docs` here.

#![cfg(target_arch = "wasm32")]

use omnia_guest::mcp::{
    self, CallToolResult, Implementation, McpError, McpServer, Resource, ResourceContents, Tool,
};
use serde::Deserialize;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

/// The compiled-in prose corpus as `(name, title, body)` triples.
const DOCS: &[(&str, &str, &str)] = &[
    (
        "overview",
        "Widget Service Overview",
        "# Widget Service Overview\n\n\
         Widgets move through `draft`, `assembled`, and `shipped` in order. They \
         never move backwards.\n",
    ),
    (
        "api-reference",
        "Widget Service API Reference",
        "# Widget Service API Reference\n\n\
         `POST /widgets` creates a draft widget. `POST /widgets/{id}/assemble` \
         advances it to `assembled`.\n",
    ),
    (
        "style-guide",
        "Widget Service Style Guide",
        "# Widget Service Style Guide\n\n\
         Labels are kebab-case. IDs are ULIDs.\n",
    ),
];

impl Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        mcp::serve(Docs, request).await
    }
}

fn find_doc(name: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    DOCS.iter().find(|(doc_name, ..)| *doc_name == name)
}

#[derive(Deserialize)]
struct ReadDocArgs {
    name: String,
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
                "List the name and title of every available document.",
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
                let ReadDocArgs { name: doc } = mcp::arguments(arguments)?;
                find_doc(&doc).map_or_else(
                    || Ok(CallToolResult::error(format!("no document named `{doc}`"))),
                    |(.., body)| Ok(CallToolResult::text(*body)),
                )
            }
            other => Err(McpError::unknown_tool(other)),
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
            || Err(McpError::resource_not_found(uri)),
            |(.., body)| Ok(ResourceContents::text(uri, "text/markdown", *body)),
        )
    }
}
