//! Without a `check`, a `format::schema` answer passes through as the
//! backend produced it: the schema steers the provider, nothing gates.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Format, Model as _, Request, SchemaFormat};
use test_programs::{VERDICT_SCHEMA, user};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let request = Request::builder()
        .messages(vec![user("hi")])
        .format(Format::Schema(
            SchemaFormat::builder().name("verdict").schema(VERDICT_SCHEMA).build(),
        ))
        .build();

    let reply = WasiProvider.complete(request).await.expect("the answer passes through");
    assert_eq!(reply.answer, r#"{"verdict":"pass"}"#);
}
