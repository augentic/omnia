//! Imports `ping` from `wit-skew`'s `omnia-test:link/ops`, which returns
//! `u32` where `full` expects `string`, so the link suite can pin that two
//! importers of one function must agree on its signature.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "skewed-importer",
    path: "wit-skew",
});

struct SkewedImporter;

export!(SkewedImporter);

impl Guest for SkewedImporter {
    fn poke(message: String) -> u32 {
        crate::omnia_test::link::ops::ping("echoer", &message)
    }
}
