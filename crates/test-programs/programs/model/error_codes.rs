//! Every `omnia:model` outcome reaches the guest by its case: `invalid` is
//! `InvalidRequest`, `exhausted` is `BudgetExhausted`, `other` is `Backend`,
//! and every other case is the variant of its name. The suite's faulting
//! backend fails a completion by the model it names. The host refuses an
//! empty model name before it reaches the backend, and fails a session
//! answered with the wrong kind of result.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Error, Model as _, Reply, Request};
use omnia_wasi_model::omnia::{completion, types};
use omnia_wasi_model::wit_stream;
use test_programs::{raw_lookup, raw_model, raw_request, user};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    // the backend's chosen cases, through the SDK
    for name in
        ["not-found", "denied", "invalid", "unsupported", "unavailable", "other", "exhausted"]
    {
        let request = Request::builder().model(name).messages(vec![user("hi")]).build();
        assert_eq!(code(WasiProvider.complete(request).await), name);
    }

    // the host's own rules
    assert_eq!(case(completion::open(String::new()).await), "invalid");

    // a verdict for a tool call
    let model = raw_model().await;
    let (mut results, results_rx) = wit_stream::new();
    let session = model
        .complete(raw_request(vec![raw_lookup()], no_grants()), results_rx)
        .await
        .expect("session opens");
    let completion::Session { mut calls, reply } = session;
    let Some(types::Call::Tool(call)) = calls.next().await else {
        panic!("model issues a tool call");
    };
    let verdict = types::Verdict {
        id: call.id,
        outcome: Ok(()),
    };
    let _ = results.write_one(types::CallResult::Check(verdict)).await;
    assert_eq!(case(reply.await), "invalid");
}

const fn no_grants() -> types::Grants<'static> {
    types::Grants {
        workspace: None,
        worker: None,
    }
}

// the `omnia:model` case an SDK failure reports
fn code(result: Result<Reply, Error>) -> &'static str {
    match result.expect_err("the backend fails the completion") {
        Error::InvalidRequest(_) => "invalid",
        Error::BudgetExhausted(_) => "exhausted",
        Error::NotFound(_) => "not-found",
        Error::Denied(_) => "denied",
        Error::Unsupported(_) => "unsupported",
        Error::Unavailable(_) => "unavailable",
        Error::Backend(_) => "other",
    }
}

fn case<T: Debug>(result: Result<T, types::Error>) -> &'static str {
    match result.expect_err("the host refuses the call") {
        types::Error::NotFound(_) => "not-found",
        types::Error::Denied(_) => "denied",
        types::Error::Invalid(_) => "invalid",
        types::Error::Unsupported(_) => "unsupported",
        types::Error::Unavailable(_) => "unavailable",
        types::Error::Other(_) => "other",
    }
}
