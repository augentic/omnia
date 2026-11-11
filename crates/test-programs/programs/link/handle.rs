//! Imports `omnia-test:link/tokens`, whose `take` carries a resource inside
//! an optional record, so the link suite can pin the refusal of a signature
//! no plain value crosses.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "handle",
    path: "wit",
});

struct Handle;

export!(Handle);

impl Guest for Handle {
    // the call keeps `take` from being pruned at componentization
    fn poke() {
        omnia_test::link::tokens::take(None);
    }
}
