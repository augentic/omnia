//! A backend that finishes without an accepted candidate leaves the question
//! unanswered, failing with the backend error its first argument names.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Error, Question};
use test_programs::Verdict;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let expected = std::env::args().nth(1).expect("the expected backend error");
    let error = Question::<Verdict>::new("verdict")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect_err("no candidate was accepted");
    let Error::Backend(detail) = error else {
        panic!("expected backend, got {error:?}");
    };
    assert_eq!(detail, expected);
}
