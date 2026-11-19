//! Raw session bindings: dropping the results writer before answering is
//! `invalid`, not a timeout.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::omnia::{completion, types};
use omnia_wasi_model::wit_stream;
use test_programs::{raw_lookup, raw_model, raw_request};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let request = raw_request(
        vec![raw_lookup()],
        types::Grants {
            workspace: None,
            worker: None,
        },
    );
    let model = raw_model().await;
    let (results, results_rx) = wit_stream::new();
    let session = model.complete(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let Some(types::Call::Tool(call)) = calls.next().await else {
        panic!("model issues a tool call");
    };
    assert_eq!(call.name, "lookup");

    drop(results);

    let error = reply.await.expect_err("closed results stream fails the completion");
    assert!(
        matches!(error, types::Error::Invalid(ref detail) if detail.contains("closed its results stream")),
        "unexpected: {error:?}"
    );
}
