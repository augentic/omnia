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

#[derive(Clone, Default, Provider)]
struct PrivateWithPrivateField {
    #[provider(Config)]
    config: PrivateConfig,
}

/// Configuration a public provider can delegate to.
#[derive(Clone, Default, Provider)]
pub struct PublicConfig {
    #[provider(Config)]
    inner: MapConfig,
}

/// Public provider delegating to a public field type.
#[derive(Clone, Default, Provider)]
pub struct PublicWithPublicField {
    #[provider(Config)]
    config: PublicConfig,
}

/// Public generic provider combining a generic field with a concrete field,
/// without a `private_bounds` predicate on either.
#[derive(Clone, Default, Provider)]
pub struct GenericWithConcreteField<S> {
    #[provider(Config)]
    config: PublicConfig,
    #[provider(StateStore)]
    storage: S,
}

#[tokio::test]
async fn generic_multi() {
    let provider = Generic {
        config: MapConfig::default().with([("k", "v")]),
        storage: Memory::default(),
    };
    provider.storage.insert_state("s", b"1");
    provider.storage.insert_object("b", "o", b"2");

    assert_eq!(Config::get(&provider, "k").await.expect("config"), "v");
    assert_eq!(StateStore::get(&provider, "s").await.expect("state").as_deref(), Some(&b"1"[..]));
    assert_eq!(BlobStore::get(&provider, "b", "o").await.expect("blob"), Some(b"2".to_vec()));

    let private = PrivateWithPrivateField {
        config: PrivateConfig {
            inner: MapConfig::default().with([("p", "ok")]),
        },
    };
    assert_eq!(Config::get(&private, "p").await.expect("private field config"), "ok");

    let public = PublicWithPublicField {
        config: PublicConfig {
            inner: MapConfig::default().with([("q", "ok")]),
        },
    };
    assert_eq!(Config::get(&public, "q").await.expect("public field config"), "ok");

    let mixed = GenericWithConcreteField {
        config: PublicConfig {
            inner: MapConfig::default().with([("m", "mixed-ok")]),
        },
        storage: Memory::default(),
    };
    mixed.storage.insert_state("ms", b"9");
    assert_eq!(Config::get(&mixed, "m").await.expect("mixed config"), "mixed-ok");
    assert_eq!(
        StateStore::get(&mixed, "ms").await.expect("mixed state").as_deref(),
        Some(&b"9"[..])
    );
}

#[test]
fn compile_fail_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/duplicate_capability.rs");
    cases.compile_fail("tests/ui/unknown_capability.rs");
}
