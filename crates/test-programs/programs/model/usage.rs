//! A backend-reported usage record is projected onto the guest reply.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Model as _, Request, Usage, WasiModel};
use test_programs::user;

test_programs::run!(scenario);

async fn scenario() {
    let reply = WasiModel
        .complete(Request::builder().messages(vec![user("hi")]).build())
        .await
        .expect("canned answer carries usage");
    assert_eq!(reply.answer, "hi");
    assert_eq!(
        reply.usage,
        Some(Usage {
            input_tokens: 3,
            output_tokens: 5,
            reasoning_tokens: Some(1),
        })
    );
}
