//! Imports both `omnia-test:link/ops` functions, so its dispatch wiring must
//! extend whatever subset an earlier-assembled importer already wired.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "full",
    path: "wit",
});

struct Full;

export!(Full);

impl Guest for Full {
    fn poke(message: String) -> String {
        omnia_test::link::ops::ping("echoer", &message)
    }

    async fn poke_async(message: String) -> String {
        omnia_test::link::ops::ping_async("echoer".to_owned(), message).await
    }
}
