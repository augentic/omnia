//! The echo default fails loud on `format::schema`; the guest sees `backend`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Error, Format, Model as _, Request, SchemaFormat};
use test_programs::{VERDICT_SCHEMA, user};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let request = Request::builder()
        .messages(vec![user("hi")])
        .format(Format::Schema(
            SchemaFormat::builder().name("verdict").schema(VERDICT_SCHEMA).build(),
        ))
        .build();

    let error = WasiProvider.complete(request).await.expect_err("echo cannot satisfy a schema");
    assert!(
        matches!(error, Error::Backend(ref detail) if detail.contains("cannot satisfy format::schema")),
        "unexpected: {error:?}"
    );
}
