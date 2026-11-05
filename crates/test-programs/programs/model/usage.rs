//! A backend-reported usage record is projected onto the guest reply.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Model as _, Request, Usage};
use test_programs::user;

omnia_sdk::export!(command: scenario);

async fn scenario() {
    let reply = WasiProvider
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
