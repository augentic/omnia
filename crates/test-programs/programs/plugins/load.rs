//! Happy path through the omnia-sdk plugin requester: argv names the source
//! the guest loads from — a declared name, a mounted path, or a release —
//! and, optionally, the digest its bytes must hash to. Two loads race for
//! the first admission and both succeed with one name and one digest; the
//! plugin's name routes host-mediated dispatch to the loaded exporter; a
//! later load is idempotent — pinned to the resolved digest where the source
//! takes a pin. The loader import arrives through the SDK's own bindings;
//! this world only imports `ops`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{self, File, Release, Source};

use crate::omnia_test::link::ops;

omnia_sdk::command::export!(scenario);

// `declared`, `path`, `release`, or `release@<registry>` — the last naming
// the registry the load fetches from.
fn source(kind: &str, value: &str, pin: Option<String>) -> Source {
    let value = value.to_owned();
    let (kind, registry) = kind
        .split_once('@')
        .map_or((kind, None), |(kind, registry)| (kind, Some(registry.to_owned())));
    match (kind, registry) {
        ("declared", None) => Source::Declared(value),
        ("path", None) => Source::Path(File { path: value, pin }),
        ("release", registry) => Source::Release(Release {
            reference: value,
            registry,
            pin,
        }),
        _ => panic!("expected `declared`, `path`, `release`, or `release@<registry>`"),
    }
}

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (kind, value, expected) = match arguments.as_slice() {
        [_, kind, value] => (kind, value, None),
        [_, kind, value, digest] => (kind, value, Some(digest)),
        _ => panic!("expected `<declared|path|release> <value> [digest]`; got {arguments:?}"),
    };
    let from = source(kind, value, None);
    let name = from.name().to_owned();

    // Two requests for one source: whichever admits first, the other
    // attests the same registration.
    let (first, second) = futures::join!(plugins::load(from.clone()), plugins::load(from.clone()));
    let first = first.expect("the first load runs").expect("the first load succeeds");
    let second = second.expect("the racing load runs").expect("the racing load succeeds");
    assert_eq!(first.name, name, "a guest registers under its source's name");
    assert_eq!(second.name, name);
    assert_eq!(second.digest, first.digest, "both plugins attest one registration");
    if let Some(expected) = expected {
        assert_eq!(&first.digest, expected, "the reported digest is the expected one");
    }

    // The plugin lists what the component exports, so a caller can hold it
    // to the contract it expects before dispatching.
    assert!(
        first.exports.iter().any(|export| export == "omnia-test:link/ops"),
        "the plugin lists the exported interface: {:?}",
        first.exports
    );
    assert_eq!(second.exports, first.exports);

    // The plugin's name routes host-mediated dispatch to the exporter.
    let answer = ops::ping(&first.name, "hi");
    assert_eq!(answer, format!("{name} pong: hi"));

    // A declared entry carries its own pin; a path or a release takes the
    // digest the first load reported.
    let pin = (!matches!(from, Source::Declared(_))).then(|| first.digest.clone());
    let again = plugins::load(source(kind, value, pin)).await;
    let again = again.expect("a later load runs").expect("a later load attests");
    assert_eq!(again.name, name);
    assert_eq!(again.digest, first.digest);
}
