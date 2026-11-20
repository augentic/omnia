//! A backend that finishes without an accepted candidate leaves the question
//! unanswered, failing as `other` with the detail its first argument names.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Question;
use test_programs::Verdict;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let expected = std::env::args().nth(1).expect("the expected detail");
    let error = Question::<Verdict>::new("verdict")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect_err("no candidate was accepted");
    assert_eq!((error.code(), error.description()), ("other", expected.as_str()));
}
