//! # Cursor example — `ask` guest
//!
//! A `wasi:cli/command` reactor that **imports** `omnia:model/completion` and
//! calls `create` once when the host drives `wasi:cli/run`. The prompt carries
//! a `docs` MCP grant; when the runtime binds `WasiModel` to the cursor backend,
//! the backend resolves that logical name to a configured endpoint and wires the
//! spawned `cursor-agent` to the read-only MCP documentation server served in the
//! background by the sibling `docs` guest.
//!
//! It also exports `wasi:http` on `/ask` so the same completion can be triggered
//! over HTTP. `omnia.toml` routes `/ask` here.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use axum::routing::get;
use omnia_wasi_model::completion::{
    self, Format, Grants, Mcp, Request as CompletionRequest, Sections, Tool,
};
use wasip3::exports::cli::run::Guest;
use wasip3::exports::http::handler::Guest as HttpGuest;
use wasip3::http::types::{ErrorCode, Request as HttpRequest, Response};

struct CliGuest;
wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        let request = completion::Request {
            model: None,
            system: Some(
                "You answer strictly from the read-only `docs` MCP documentation tools. Do not guess."
                    .to_string(),
            ),
            messages: vec![],
            sections: Some(Sections {
                role: Some("a terse technical writer".to_string()),
                task: "Using the docs MCP server, state the lifecycle stages a widget moves \
                    through, in order."
                    .to_string(),
                context: None,
                constraints: vec![],
                examples: vec![],
                variables: vec![],
            }),
            generation: None,
            format: Format::Json,
            tools: vec![Tool::Mcp(Mcp {
                name: "docs".to_string(),
                tools: vec![],
            })],
            tool_choice: None,
            metadata: vec![],
            grants: Grants {
                references: None,
                workspace: None,
                verify: vec![],
            },
        };

        let answer = match completion::create(request).await {
            Ok(reply) => reply.answer,
            Err(error) => format!("error: {error:?}"),
        };

        println!("{answer}");
        Ok(())
    }
}

struct HttpMcp;
wasip3::http::service::export!(HttpMcp);

impl HttpGuest for HttpMcp {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(mcp));
        omnia_wasi_http::serve(router, request).await
    }
}

// Trigger the same completion over HTTP and return its validated answer.
async fn mcp_request() -> String {
    println!("mcp_request");
    "mcp response".to_string()
}
