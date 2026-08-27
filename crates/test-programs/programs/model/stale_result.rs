//! Raw session bindings: a result with an unknown id is dropped, not fatal.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::{completion, wit_stream};
use test_programs::{raw_lookup, raw_request};

test_programs::run!(scenario);

async fn scenario() {
    let request = raw_request(vec![raw_lookup()], completion::Grants { workspace: None });
    let (mut results, results_rx) = wit_stream::new();
    let session = completion::create(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let call = calls.next().await.expect("model issues a tool call");
    let _ = results
        .write_one(completion::ToolResult {
            id: "nope".to_owned(),
            output: Ok("ignored".to_owned()),
        })
        .await;
    let _ = results
        .write_one(completion::ToolResult {
            id: call.id.clone(),
            output: Ok("42".to_owned()),
        })
        .await;

    let outcome = reply.await;
    assert_eq!(outcome.expect("unknown id is dropped").answer, "42");
}
