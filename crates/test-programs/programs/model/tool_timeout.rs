//! Raw session bindings: an unanswered tool call hits the host's per-call
//! timeout while the guest still holds the results writer open.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::{completion, wit_stream};
use test_programs::{raw_lookup, raw_request};

test_programs::run!(scenario);

async fn scenario() {
    let request = raw_request(vec![raw_lookup()], completion::Grants { workspace: None });
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
}
