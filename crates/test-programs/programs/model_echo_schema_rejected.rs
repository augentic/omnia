//! The echo default fails loud on `format::schema`; the guest sees `backend`.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Error, Format, Model as _, Request, SchemaFormat, WasiModel};
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

        let error = WasiModel.complete(request).await.expect_err("echo cannot satisfy a schema");
        assert!(
            matches!(error, Error::Backend(ref detail) if detail.contains("cannot satisfy format::schema")),
            "unexpected: {error:?}"
        );
        Ok(())
    }
}
