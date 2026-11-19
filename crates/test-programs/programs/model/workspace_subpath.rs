//! Lending a subdirectory of the mount (not the root) reopens beneath it.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::user;
use wasip3::filesystem::preopens;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    // the `./` prefix, so `.` is the preopen rather than a stray character of `nested`
    let request =
        Request::builder().messages(vec![user("workspace")]).workspace("./nested").build();
    let reply = request.complete().await.expect("subpath tools answer");
    assert_eq!(reply.answer, "hello:out.txt,seed.txt");
}
