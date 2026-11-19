//! Raw session bindings: an unanswered tool call hits the host's per-call
//! timeout while the guest still holds the results writer open.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{completion, types};
use omnia_sdk::wit_stream;
use test_programs::{raw_lookup, raw_model, raw_request};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let request = raw_request(vec![raw_lookup()], types::Grants { workspace: None });
    let model = raw_model().await;
    let (results, results_rx) = wit_stream::new();
    let session = model.complete(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let Some(types::Call::Tool(call)) = calls.next().await else {
        panic!("model issues a tool call");
    };
    assert_eq!(call.name, "lookup");

    // Never answer; holding the results writer open forces the host's
    // per-call timeout rather than a closed-stream failure.
    let outcome = reply.await;
    drop(results);

    let exhausted =
        outcome.expect("a timeout is an outcome").expect_err("unanswered call times out");
    assert!(matches!(exhausted.budget, types::Budget::CallTime), "unexpected: {exhausted:?}");
    assert!(exhausted.detail.contains("no result within"), "unexpected: {exhausted:?}");
}
