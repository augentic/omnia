//! Exports `omnia-test:link/ops` and answers each call with the level its
//! subscriber opened at — the one the chain it was dispatched on carries —
//! reloading nothing itself. `ping-async` answers from inside an INFO span,
//! so the host sees that span exactly when the chain names `info` or finer.
//! No trigger of its own — instantiated fresh per dispatched call.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "echoer",
    path: "wit",
});

struct Echoer;

export!(Echoer);

impl exports::omnia_test::link::ops::Guest for Echoer {
    fn ping(_target: String, _message: String) -> String {
        omnia_wasi_otel::level().to_string()
    }

    async fn ping_async(_target: String, _message: String) -> String {
        opened().await
    }
}

#[omnia_wasi_otel::instrument]
async fn opened() -> String {
    omnia_wasi_otel::level().to_string()
}
