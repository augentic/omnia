//! Imports `omnia-test:versioned/ops` at 0.1.0 and exports it at 0.1.1, two
//! names wasmtime's semver-compatible lookup takes for one, so the link suite
//! can pin that a call reaches an export by its exact name.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "versioned",
    path: "wit-versioned",
    generate_all,
});

struct Versioned;

export!(Versioned);

impl exports::omnia_test::versioned0_1_1::ops::Guest for Versioned {
    fn ping(target: String, message: String) -> String {
        format!("{target} pong: {message}")
    }
}

impl Guest for Versioned {
    fn poke(message: String) -> String {
        crate::omnia_test::versioned0_1_0::ops::ping("versioned", &message)
    }
}
