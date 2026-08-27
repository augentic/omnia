//! A conforming backend answer passes the schema gate and reaches the guest.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Format, Model as _, Request, SchemaFormat, WasiModel};
use test_programs::{VERDICT_SCHEMA, user};
use wasip3::exports::cli::run::Guest;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        let request = Request::builder()
            .messages(vec![user("hi")])
            .format(Format::Schema(
                SchemaFormat::builder().name("verdict").schema(VERDICT_SCHEMA).build(),
            ))
            .build();

        let reply = WasiModel.complete(request).await.expect("canned answer satisfies the schema");
        assert_eq!(reply.answer, r#"{"verdict":"pass"}"#);
        Ok(())
    }
}
