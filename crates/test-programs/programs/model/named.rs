//! A guest completes on the models the deployment declares by name. A
//! request that names none opens `default`, and an undeclared name is
//! `not-found`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::{fails_with, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    for name in [Some("fast"), None] {
        let request = Request::builder().maybe_model(name).messages(vec![user("hi")]).build();
        let reply = request
            .complete()
            .await
            .expect("the completion runs")
            .expect("a declared model answers");
        assert_eq!(reply.answer, "hi", "{name:?}");
    }

    let request = Request::builder().model("slow").messages(vec![user("hi")]).build();
    let error = request.complete().await.expect_err("an undeclared model fails");
    fails_with(&error, "not-found", "`slow`");
}
