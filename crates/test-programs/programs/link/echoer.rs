//! Exports `omnia-test:link/ops`: the dispatch target for the link suite.
//! No trigger of its own — instantiated fresh per dispatched call.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "echoer",
    path: "wit",
});

struct Echoer;

export!(Echoer);

impl exports::omnia_test::link::ops::Guest for Echoer {
    fn ping(target: String, message: String) -> String {
        format!("{target} pong: {message}")
    }

    async fn ping_async(target: String, message: String) -> String {
        format!("{target} pong-async: {message}")
    }
}
