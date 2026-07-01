//! # Model example — `complete` guest
//!
//! Imports `omnia:model/completion` and calls `complete` once with a
//! `json-schema` prompt assembled from `sections`. It declares no HTTP/messaging
//! trigger, so it is driven by the integration test (`crates/wasi-model/tests`)
//! rather than a live request — the run-1 (replay) acceptance vehicle (§6).
//!
//! `run` is `async` because `complete` is an async import. The guest sets
//! `grants.references = "shelf"` as data, but Phase 1 replay short-circuits tool
//! calls, so no `resolve` (and no `shelf` guest) is exercised here — that lands
//! in Phase 2a.
//!
//! It also reads `wasi:filesystem/preopens` and, when the host has mounted a
//! workspace named `.` (the `[[mount]]` in `omnia.toml`), lends it
//! through `grants.workspace`. With no mount configured the preopen table is
//! empty and the guest lends nothing.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "complete",
    path: "model/wit",
    generate_all,
});

use omnia::model::completion;
use wasi::filesystem::preopens;

struct Example;

export!(Example);

impl Guest for Example {
    async fn run() -> String {
        // Read the preopen table the host populated from `[[mount]]` (RFC-55) and
        // pick the tree named `.` to lend. `directories` must outlive the
        // `complete` call below — the lent `workspace` borrows one of its
        // descriptors.
        let directories = preopens::get_directories();
        let workspace = directories.iter().find_map(|(dir, name)| (name == ".").then_some(dir));

        let prompt = completion::Prompt {
            model: None,
            system: None,
            messages: vec![],
            sections: Some(completion::Sections {
                role: Some("a terse code reviewer".to_string()),
                task: "decide whether the change is acceptable".to_string(),
                context: Some("the diff adds a bounds check".to_string()),
                constraints: vec![],
                examples: vec![],
                variables: vec![],
            }),
            generation: None,
            response_format: completion::ResponseFormat {
                kind: completion::ResponseFormatKind::JsonSchema,
                json_schema: Some(completion::JsonSchemaSpec {
                    name: "verdict".to_string(),
                    schema: "{\"type\":\"object\"}".to_string(),
                    strict: None,
                }),
            },
            tools: vec![],
            tool_choice: None,
            metadata: vec![],
            grants: completion::ToolGrants {
                references: Some("shelf".to_string()),
                workspace,
                verify: vec![],
            },
        };

        match completion::complete(prompt).await {
            Ok(answer) => answer,
            Err(error) => format!("error: {error:?}"),
        }
    }
}
