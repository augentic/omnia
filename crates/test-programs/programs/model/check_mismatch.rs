//! The candidate never deserializes as `T`: the schema and the type
//! disagree, which the guest sees as `invalid`, not more rounds.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Question;
use test_programs::{Verdict, fails_with};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let error = Question::<Verdict>::new("verdict")
        .ask("judge this", None, |_: &Verdict| {
            panic!("a candidate that does not deserialize never reaches the check")
        })
        .await
        .expect_err("the candidate does not deserialize");
    fails_with(&error, "invalid", "schema and answer type disagree");
}
