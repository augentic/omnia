//! Exports `omnia-test:link/ops` and answers each call with the entry of the
//! baggage it was dispatched with that the message names, empty when there is
//! none. No trigger of its own — instantiated fresh per dispatched call.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "echoer",
    path: "wit",
});

struct Echoer;

export!(Echoer);

impl exports::omnia_test::link::ops::Guest for Echoer {
    fn ping(_target: String, name: String) -> String {
        entry(&name)
    }

    async fn ping_async(_target: String, name: String) -> String {
        entry(&name)
    }
}

fn entry(name: &str) -> String {
    omnia_wasi_otel::baggage().get(name).map(|value| value.as_str().to_owned()).unwrap_or_default()
}
