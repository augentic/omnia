//! # Cursor example — `ask` guest
//!
//! Imports `omnia:model/completion` and, on an inbound HTTP request, calls
//! `complete` once. When the host binds `WasiModel` to the cursor backend with
//! `CURSOR_MCP_URL` set, the spawned `cursor-agent` answers by calling the
//! read-only MCP documentation server served by the sibling `docs` guest.
//! `omnia.toml` routes `/ask` here.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use axum::routing::get;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

mod bindings {
    #![allow(missing_docs)]
    // `completion` borrows a `wasi:filesystem` descriptor for `grants.workspace`.
    // Reuse `wasip3`'s already-generated filesystem/clocks bindings so this guest
    // and the `wasip3` HTTP export agree on one `wasi:filesystem@0.3.0` (a
    // mismatched vendored copy fails componentization).
    wit_bindgen::generate!({
        world: "ask",
        path: "cursor/wit",
        with: {
            "omnia:model/completion@0.1.0": generate,
            "wasi:filesystem/types@0.3.0": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.0": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.0": wasip3::clocks::types,
        },
    });
}

use bindings::omnia::model::completion;

struct AskGuest;
wasip3::http::service::export!(AskGuest);

impl Guest for AskGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(ask));
        omnia_wasi_http::serve(router, request).await
    }
}

async fn ask() -> String {
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

    match completion::complete(prompt).await {
        Ok(answer) => answer,
        Err(error) => format!("error: {error:?}"),
    }
}
