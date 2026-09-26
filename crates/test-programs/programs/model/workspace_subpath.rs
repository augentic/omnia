//! Lending a subdirectory of the mount (not the root) reopens beneath it.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Model as _, Request, WasiModel};
use test_programs::user;
use wasip3::filesystem::preopens;

omnia_sdk::command!(scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    let reply = WasiModel
        .complete(
            // the `./` prefix, so `.` is the preopen rather than a stray character of `nested`
            Request::builder().messages(vec![user("workspace")]).workspace("./nested").build(),
        )
        .await
        .expect("subpath tools answer");
    assert_eq!(reply.answer, "hello:out.txt,seed.txt");
}
