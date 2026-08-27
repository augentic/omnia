//! Lending the mounted workspace lets the backend drive the host-injected
//! `read`/`write`/`list` tools against it.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Model as _, Request, WasiModel};
use test_programs::user;
use wasip3::exports::cli::run::Guest;
use wasip3::filesystem::preopens;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        assert!(
            preopens::get_directories().iter().any(|(_, name)| name == "."),
            "host must mount `.`"
        );

        let reply = WasiModel
            .complete(Request::builder().messages(vec![user("workspace")]).workspace(".").build())
            .await
            .expect("workspace tools answer");
        // seed.txt content, then the sorted listing after the backend's write.
        assert_eq!(reply.answer, "hello:out.txt,seed.txt");
        Ok(())
    }
}
