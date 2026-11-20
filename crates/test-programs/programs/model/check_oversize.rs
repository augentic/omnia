//! A correction over the host's byte cap fails the completion as `invalid`,
//! like an oversize tool result: the check's text is bound for the model too.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Question;
use test_programs::{Verdict, fails_with};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let error = Question::<Verdict>::new("verdict")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect_err("the correction exceeds the cap");
    fails_with(&error, "invalid", "exceeds the 4-byte cap");
}
