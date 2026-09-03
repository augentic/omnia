//! Raw session bindings: two parallel calls answered in reverse order still
//! correlate by id, not arrival order.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::{completion, wit_stream};
use test_programs::{raw_lookup, raw_request};

omnia_guest::command!(scenario);

async fn scenario() {
    let request = raw_request(vec![raw_lookup()], completion::Grants { workspace: None });
    let (mut results, results_rx) = wit_stream::new();
    let session = completion::create(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let first = calls.next().await.expect("first call");
    let second = calls.next().await.expect("second call");
    assert_ne!(first.id, second.id);

    // Answer in reverse order; each output echoes its call's arguments,
    // so the backend's in-order reply proves correlation by id.
    let _ = results
        .write_one(completion::ToolResult {
            id: second.id.clone(),
            output: Ok(second.arguments.clone()),
        })
        .await;
    let _ = results
        .write_one(completion::ToolResult {
            id: first.id.clone(),
            output: Ok(first.arguments.clone()),
        })
        .await;

    let outcome = reply.await;
    assert_eq!(outcome.expect("both calls resolve").answer, "1|2");
}
