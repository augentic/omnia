//! A handler `Err` is repair input the model sees, not a session failure.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Model as _, Request, WasiModel};
use test_programs::{lookup, user};
use wasip3::exports::cli::run::Guest;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();

        let reply = WasiModel
            .complete_with(request, |_call| async { Err::<String, _>("no data".to_owned()) })
            .await
            .expect("the model turns the failure into an answer");
        assert_eq!(reply.answer, "tool failed: no data");
        Ok(())
    }
}
