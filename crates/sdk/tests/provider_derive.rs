//! `#[derive(Provider)]` delegation and compile-fail diagnostics.

#![cfg(not(target_arch = "wasm32"))]
#![deny(private_bounds, private_interfaces)]

use omnia_sdk::{BlobStore, Config, Provider, StateStore};
use omnia_test::guest::{MapConfig, Memory};

#[derive(Clone, Default, Provider)]
struct Generic<C, S> {
    #[provider(Config)]
    config: C,
    #[provider(StateStore, BlobStore)]
    storage: S,
}

#[derive(Clone, Default, Provider)]
struct PrivateConfig {
    #[provider(Config)]
    inner: MapConfig,
}

/// Public provider delegating to a private field type without triggering
/// `private_bounds`.
#[derive(Clone, Default, Provider)]
pub struct PublicWithPrivateField {
    #[provider(Config)]
    config: PrivateConfig,
}

/// Public generic provider combining a generic field with a private concrete
/// field without emitting a `private_bounds` predicate on the private field.
#[derive(Clone, Default, Provider)]
pub struct GenericWithPrivateField<S> {
    #[provider(Config)]
    config: PrivateConfig,
    #[provider(StateStore)]
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

    let pub_provider = PublicWithPrivateField {
        config: PrivateConfig {
            inner: MapConfig::default().with([("p", "ok")]),
        },
    };
    assert_eq!(Config::get(&pub_provider, "p").await.expect("private field config"), "ok");

    let mixed = GenericWithPrivateField {
        config: PrivateConfig {
            inner: MapConfig::default().with([("m", "mixed-ok")]),
        },
        storage: Memory::default(),
    };
    mixed.storage.insert_state("ms", b"9");
    assert_eq!(Config::get(&mixed, "m").await.expect("mixed config"), "mixed-ok");
    assert_eq!(StateStore::get(&mixed, "ms").await.expect("mixed state"), Some(b"9".to_vec()));
}

#[test]
fn compile_fail_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/duplicate_capability.rs");
    cases.compile_fail("tests/ui/unknown_capability.rs");
}
