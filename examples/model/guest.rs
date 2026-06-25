//! # Model example — `complete` guest
//!
//! Imports `augentic:model/completion` and calls `complete` once with a
//! `json-schema` prompt assembled from `sections`. It declares no HTTP/messaging
//! trigger, so it is driven by the integration test (`crates/wasi-model/tests`)
//! rather than a live request — the run-1 (replay) acceptance vehicle (§6).
//!
//! `run` is `async` because `complete` is an async import. The guest sets
//! `grants.references = "shelf"` as data, but Phase 1 replay short-circuits tool
//! calls, so no `resolve` (and no `shelf` guest) is exercised here — that lands
//! in Phase 2a.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "complete",
    path: "model/wit",
    generate_all,
});

use augentic::model::completion;

struct Example;

export!(Example);

impl Guest for Example {
    async fn run() -> String {
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
                working_tree: None,
                verify: vec![],
            },
        };

        match completion::complete(prompt).await {
            Ok(answer) => answer,
            Err(error) => format!("error: {error:?}"),
        }
    }
}
