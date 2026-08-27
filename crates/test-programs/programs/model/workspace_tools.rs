//! Lending the mounted workspace lets the backend drive the host-injected
//! `read`/`write`/`list` tools against it.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Model as _, Request, WasiModel};
use test_programs::user;
use wasip3::filesystem::preopens;

test_programs::run!(scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    let reply = WasiModel
        .complete(Request::builder().messages(vec![user("workspace")]).workspace(".").build())
        .await
        .expect("workspace tools answer");
    // seed.txt content, then the sorted listing after the backend's write.
    assert_eq!(reply.answer, "hello:out.txt,seed.txt");
}
