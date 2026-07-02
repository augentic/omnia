//! # Cursor example — `ask` guest
//!
//! A `wasi:cli/command` reactor that **imports** `omnia:model/completion` and
//! calls `complete` once when the host drives `wasi:cli/run`. When the runtime
//! binds `WasiModel` to the cursor backend with `CURSOR_MCP_URL` set, the spawned
//! `cursor-agent` answers by calling the read-only MCP documentation server
//! served in the background by the sibling `docs` guest.
//!
//! It also exports `wasi:http` on `/ask` so the same completion can be triggered
//! over HTTP. `omnia.toml` routes `/ask` here.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use axum::routing::get;
use omnia_wasi_model::completion;
use wasip3::exports::cli::run::Guest;
use wasip3::exports::http::handler::Guest as HttpGuest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct CmdGuest;
wasip3::cli::command::export!(CmdGuest);

impl Guest for CmdGuest {
    async fn run() -> Result<(), ()> {
        // build a prompt
        let prompt = completion::Prompt {
            model: None,
            system: Some(
                "You answer strictly from the read-only `omnia` MCP documentation tools \
             (`list_docs`, `read_doc`). Do not guess."
                    .to_string(),
            ),
            messages: vec![],
            sections: Some(completion::Sections {
                role: Some("a terse technical writer".to_string()),
                task: "Using the omnia MCP docs, state the lifecycle stages a widget moves \
                   through, in order."
                    .to_string(),
                context: None,
                constraints: vec![],
                examples: vec![],
                variables: vec![],
            }),
            generation: None,
            response_format: completion::ResponseFormat {
                kind: completion::ResponseFormatKind::JsonObject,
                json_schema: None,
            },
            tools: vec![],
            tool_choice: None,
            metadata: vec![],
            grants: completion::ToolGrants {
                references: None,
                workspace: None,
                verify: vec![],
            },
        };

        // call the model (cursor-agent)
        let answer = match completion::complete(prompt).await {
            Ok(answer) => answer,
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
        let router = Router::new().route("/", get(mcp_docs));
        omnia_wasi_http::serve(router, request).await
    }
}

async fn mcp_docs() -> String {
    let doc= "A simple MCP server that returns a  referencedocument .";
}
