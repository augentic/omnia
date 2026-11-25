//! Imports `omnia-test:link/counter`, whose `count` leads with a number
//! rather than a guest identity, so the link suite can pin the relay's
//! refusal of a call that names no target.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "anonymous",
    path: "wit",
});

struct Anonymous;

export!(Anonymous);

impl Guest for Anonymous {
    fn poke(message: String) -> String {
        let n = crate::omnia_test::link::counter::count(7);
        format!("{message}: {n}")
    }
}
