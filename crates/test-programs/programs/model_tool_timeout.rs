//! Raw session bindings: an unanswered tool call hits the host's per-call
//! timeout while the guest still holds the results writer open.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::{completion, wit_stream};
use wasip3::exports::cli::run::Guest;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        let request = completion::Request {
            model: None,
            system: None,
            messages: vec![completion::Message {
                role: completion::Role::User,
                content: "hi".to_owned(),
            }],
            generation: None,
            format: completion::Format::Text,
            tools: vec![completion::Tool::Function(completion::Function {
                name: "lookup".to_owned(),
                description: "test lookup".to_owned(),
                parameters: "{}".to_owned(),
            })],
            grants: completion::Grants { workspace: None },
        };

        let (results, results_rx) = wit_stream::new();
        let session = completion::create(request, results_rx).await.expect("session opens");
        let completion::Session { mut calls, reply } = session;

        let call = calls.next().await.expect("model issues a tool call");
        assert_eq!(call.name, "lookup");

        // Never answer; holding the results writer open forces the host's
        // per-call timeout rather than a closed-stream failure.
        let outcome = reply.await;
        drop(results);

        let error = outcome.expect_err("unanswered call times out");
        assert!(
            matches!(error, completion::Error::BudgetExhausted(ref detail) if detail.contains("no result within")),
            "unexpected: {error:?}"
        );
        Ok(())
    }
}
