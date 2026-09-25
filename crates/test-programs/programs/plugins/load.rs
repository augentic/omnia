//! Happy path through the omnia-sdk plugin requester: argv names where the
//! guest loads from — a declared name, a mounted path, or a package — and,
//! optionally, the digest its bytes must hash to. Two loads race for the
//! first admission and both succeed with one identity and one digest; the
//! handle's identity routes host-mediated dispatch to the loaded exporter; a
//! later load is idempotent — pinned to the resolved digest where the
//! location takes a pin. The loader import arrives through the SDK's own
//! bindings; this world only imports `ops`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{Digest, Location, Plugins as _, WasiPlugins};
use omnia_test::link::ops;

omnia_sdk::command!(scenario);

// `declared`, `path`, `registry`, or `registry@<endpoint>` — the last naming
// the registry the load fetches from.
fn location(kind: &str, value: &str) -> Location {
    let value = value.to_owned();
    let (kind, endpoint) = kind
        .split_once('@')
        .map_or((kind, None), |(kind, endpoint)| (kind, Some(endpoint.to_owned())));
    match (kind, endpoint) {
        ("declared", None) => Location::Declared(value),
        ("path", None) => Location::Path(value),
        ("registry", endpoint) => Location::Registry {
            package: value,
            endpoint,
        },
        _ => panic!("expected `declared`, `path`, `registry`, or `registry@<endpoint>`"),
    }
}

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (from, expected) = match arguments.as_slice() {
        [_, kind, value] => (location(kind, value), None),
        [_, kind, value, digest] => {
            (location(kind, value), Some(digest.parse::<Digest>().expect("the expected digest")))
        }
        _ => panic!("expected `<declared|path|registry> <value> [digest]`; got {arguments:?}"),
    };
    let name = from.name();

    // Two requests for one location: whichever admits first, the other
    // attests the same registration.
    let (first, second) =
        futures::join!(WasiPlugins.load(&from, None), WasiPlugins.load(&from, None));
    let first = first.expect("the first load succeeds");
    let second = second.expect("the racing load succeeds");
    assert_eq!(first.id(), name, "a guest registers under its location's name");
    assert_eq!(second.id(), name);
    let digest = first.digest();
    assert_eq!(second.digest(), digest, "both handles attest one registration");
    if let Some(expected) = &expected {
        assert_eq!(digest, expected, "the reported digest is the expected one");
    }

    // The handle's identity routes host-mediated dispatch to the exporter.
    let answer = ops::ping(first.id(), "hi");
    assert_eq!(answer, format!("{name} pong: hi"));

    // A declared entry carries its own pin; a path or a package takes the
    // digest the first load reported.
    let pin = (!matches!(from, Location::Declared(_))).then_some(digest);
    let again = WasiPlugins.load(&from, pin).await.expect("a later load attests");
    assert_eq!(again.id(), name);
    assert_eq!(again.digest(), digest);
}
