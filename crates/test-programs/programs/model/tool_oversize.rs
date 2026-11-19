//! A tool's output or failure text over the host's byte cap fails the
//! completion as `invalid`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Error, Request};
use test_programs::{lookup, user};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    for (kind, outcome) in [("result", Ok("hello")), ("failure", Err("oops!"))] {
        let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();
        let error = request
            .complete_with(|_call| async move { outcome.map(str::to_owned).map_err(str::to_owned) })
            .await
            .expect_err("oversize text fails the completion");

        assert!(
            matches!(error, Error::InvalidRequest(ref detail) if detail.contains(&format!("{kind} of 5 bytes exceeds the 4-byte cap"))),
            "unexpected: {error:?}"
        );
    }
}
