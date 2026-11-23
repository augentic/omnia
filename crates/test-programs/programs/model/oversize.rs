//! Model-bound text over the host's byte cap fails the completion as
//! `invalid`: a tool's output, a tool's failure, and a check's correction
//! alike.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Question, Request};
use test_programs::{Verdict, fails_with, lookup, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    for (kind, outcome) in [("result", Ok("hello")), ("failure", Err("oops!"))] {
        let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();
        let error = request
            .complete_with(|_call| async move { outcome.map(str::to_owned).map_err(str::to_owned) })
            .await
            .expect_err("oversize text fails the completion");
        fails_with(&error, "invalid", &format!("{kind} of 5 bytes exceeds the 4-byte cap"));
    }

    let error = Question::<Verdict>::new("verdict")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect_err("the correction exceeds the cap");
    fails_with(&error, "invalid", "exceeds the 4-byte cap");
}
