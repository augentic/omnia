//! Imports only `ping` from the host-mediated `omnia-test:link/ops`; the
//! unused `ping-async` import is pruned at componentization, so this guest's
//! view of the interface is a strict subset of `full`'s.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "partial",
    path: "wit",
});

struct Partial;

export!(Partial);

impl Guest for Partial {
    fn poke(message: String) -> String {
        omnia_test::link::ops::ping("echoer", &message)
    }
}
