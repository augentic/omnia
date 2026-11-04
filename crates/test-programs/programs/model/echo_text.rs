//! The echo default answers a text completion with the last user turn.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Model as _, Request};
use test_programs::user;

omnia_sdk::command!(scenario);

async fn scenario() {
    let reply = WasiProvider
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
