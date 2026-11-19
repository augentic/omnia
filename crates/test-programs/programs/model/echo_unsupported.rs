//! The echo default refuses what it cannot honour: `format::schema` and a
//! generation control are each `unsupported`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Error, Format, Generation, Model as _, Request, SchemaFormat};
use test_programs::{VERDICT_SCHEMA, user};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let schema = Request::builder()
        .messages(vec![user("hi")])
        .format(Format::Schema(
            SchemaFormat::builder().name("verdict").schema(VERDICT_SCHEMA).build(),
        ))
        .build();
    let generation = Request::builder()
        .messages(vec![user("hi")])
        .generation(Generation::builder().temperature(0.25).build())
        .build();

    for (request, needle) in
        [(schema, "cannot satisfy format::schema"), (generation, "ignores generation controls")]
    {
        let error = WasiProvider.complete(request).await.expect_err("the echo refuses");
        assert!(
            matches!(error, Error::Unsupported(ref detail) if detail.contains(needle)),
            "unexpected: {error:?}"
        );
    }
}
