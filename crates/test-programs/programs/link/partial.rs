//! Imports only `ping` from the host-mediated `omnia-test:link/ops`; the
//! unused `ping-async` import is pruned at componentization, so this guest's
//! view of the interface is a strict subset of `full`'s.

#![cfg(target_arch = "wasm32")]

struct Partial;

impl Guest for Partial {
    fn poke(message: String) -> String {
        self::omnia_test::link::ops::ping("echoer", &message)
    }
}

wit_bindgen::generate!({
    world: "partial",
    path: "wit",
});

export!(Partial);
