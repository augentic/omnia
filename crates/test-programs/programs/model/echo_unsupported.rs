//! The echo default refuses what it cannot honour: `format::schema` and a
//! generation control are each `unsupported`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Generation, Request};
use test_programs::{VERDICT_SCHEMA, fails_with, user, verdict_format};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let schema = Request::builder()
        .messages(vec![user("hi")])
        .format(verdict_format(VERDICT_SCHEMA))
        .build();
    let generation = Request::builder()
        .messages(vec![user("hi")])
        .generation(Generation {
            temperature: Some(0.25),
            ..Generation::default()
        })
        .build();

    for (request, needle) in
        [(schema, "cannot satisfy format::schema"), (generation, "ignores generation controls")]
    {
        let error = request.complete().await.expect_err("the echo refuses");
        fails_with(&error, "unsupported", needle);
    }
}
