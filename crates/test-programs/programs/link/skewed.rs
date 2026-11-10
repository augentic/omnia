//! Exports `omnia-test:link/ops` from `wit-skew`, whose `ping` returns `u32`
//! where every importer expects `string`, so the link suite can pin the
//! serve-time signature check.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "skewed",
    path: "wit-skew",
});

struct Skewed;

export!(Skewed);

impl exports::omnia_test::link::ops::Guest for Skewed {
    fn ping(_target: String, message: String) -> u32 {
        message.len() as u32
    }

    async fn ping_async(_target: String, message: String) -> u32 {
        message.len() as u32
    }
}
