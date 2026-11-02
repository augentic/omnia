//! `#[derive(Provider)]` delegation and compile-fail diagnostics.

#![cfg(not(target_arch = "wasm32"))]

use omnia_sdk::{BlobStore, Config, Provider, StateStore};
use omnia_test::guest::{MapConfig, Memory};

#[derive(Clone, Default, Provider)]
struct Generic<C, S> {
    #[provider(Config)]
    config: C,
    #[provider(StateStore, BlobStore)]
    storage: S,
}

#[tokio::test]
async fn generic_and_multi_capability() {
    let provider = Generic {
        config: MapConfig::default().with([("k", "v")]),
        storage: Memory::default(),
    };
    provider.storage.insert_state("s", b"1");
    provider.storage.insert_object("b", "o", b"2");

    assert_eq!(Config::get(&provider, "k").await.expect("config"), "v");
    assert_eq!(StateStore::get(&provider, "s").await.expect("state"), Some(b"1".to_vec()));
    assert_eq!(BlobStore::get(&provider, "b", "o").await.expect("blob"), Some(b"2".to_vec()));
}

#[test]
fn compile_fail_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/duplicate_capability.rs");
    cases.compile_fail("tests/ui/unknown_capability.rs");
}
