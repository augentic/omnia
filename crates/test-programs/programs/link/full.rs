//! Imports both `omnia-test:link/ops` functions, so its dispatch wiring must
//! extend whatever subset an earlier-assembled importer already wired.

#![cfg(target_arch = "wasm32")]

struct Full;

impl Guest for Full {
    fn poke(message: String) -> String {
        self::omnia_test::link::ops::ping("echoer", &message)
    }

    async fn poke_async(message: String) -> String {
        self::omnia_test::link::ops::ping_async("echoer".to_owned(), message).await
    }
}

wit_bindgen::generate!({
    world: "full",
    path: "wit",
});

export!(Full);
