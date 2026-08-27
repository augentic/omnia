//! Raw session bindings: dropping the results writer before answering is a
//! backend failure, not a timeout.

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

    drop(results);

    let error = reply.await.expect_err("closed results stream fails the completion");
    assert!(
        matches!(error, completion::Error::Backend(ref detail) if detail.contains("closed its results stream")),
        "unexpected: {error:?}"
    );
}
