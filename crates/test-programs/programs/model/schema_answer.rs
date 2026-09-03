//! A conforming backend answer passes the schema gate and reaches the guest.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Format, Model as _, Request, SchemaFormat, WasiModel};
use test_programs::{VERDICT_SCHEMA, user};

omnia_guest::command!(scenario);

async fn scenario() {
    let request = Request::builder()
        .messages(vec![user("hi")])
        .format(Format::Schema(
            SchemaFormat::builder().name("verdict").schema(VERDICT_SCHEMA).build(),
        ))
        .build();

    let reply = WasiModel.complete(request).await.expect("canned answer satisfies the schema");
    assert_eq!(reply.answer, r#"{"verdict":"pass"}"#);
}
