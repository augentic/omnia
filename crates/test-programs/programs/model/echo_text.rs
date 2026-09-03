//! The echo default answers a text completion with the last user turn.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Model as _, Request, WasiModel};
use test_programs::user;

omnia_guest::command!(scenario);

async fn scenario() {
    let reply = WasiModel
        .complete(
            Request::builder()
                .system("be terse")
                .messages(vec![user("hi"), user("second")])
                .build(),
        )
        .await
        .expect("echo answers");
    assert_eq!(reply.answer, "second");
    assert!(reply.usage.is_none());
}
