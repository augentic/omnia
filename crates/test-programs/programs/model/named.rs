//! A guest completes on the models the deployment declares by name. A
//! request that names none opens `default`, and an undeclared name is
//! `not-found`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Error, Model as _, Request};
use test_programs::user;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    for name in [Some("fast"), None] {
        let request = Request::builder().maybe_model(name).messages(vec![user("hi")]).build();
        let reply = WasiProvider.complete(request).await.expect("a declared model answers");
        assert_eq!(reply.answer, "hi", "{name:?}");
    }

    let request = Request::builder().model("slow").messages(vec![user("hi")]).build();
    let error = WasiProvider.complete(request).await.expect_err("an undeclared model fails");
    assert!(
        matches!(error, Error::Backend(ref detail) if detail.starts_with("not-found: ") && detail.contains("`slow`")),
        "unexpected: {error:?}"
    );
}
