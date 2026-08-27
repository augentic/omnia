//! A backend call to an undeclared tool fails the completion loud.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Error, Model as _, Request, WasiModel};
use test_programs::user;
use wasip3::exports::cli::run::Guest;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        // No tools declared; the host rejects the backend's `lookup` call
        // before the guest ever sees it.
        let error = WasiModel
            .complete(Request::builder().messages(vec![user("hi")]).build())
            .await
            .expect_err("undeclared tool fails the completion");
        assert!(
            matches!(error, Error::ToolFailed(ref detail) if detail.contains("does not declare")),
            "unexpected: {error:?}"
        );
        Ok(())
    }
}
