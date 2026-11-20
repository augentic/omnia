//! Without a check, a `format::schema` answer passes through as the backend
//! produced it: the schema steers the provider, nothing gates.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::{VERDICT_SCHEMA, user, verdict_format};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let request = Request::builder()
        .messages(vec![user("hi")])
        .format(verdict_format(VERDICT_SCHEMA))
        .build();

    let reply =
        request.complete().await.expect("the completion runs").expect("the answer passes through");
    assert_eq!(reply.answer, r#"{"verdict":"pass"}"#);
}
