//! Happy path through the omnia-sdk plugin requester: argv names an
//! on-demand guest and, optionally, the digest the deployment declared for
//! it. Two loads race for the first admission and both succeed with one
//! identity and one digest; the handle's identity routes host-mediated
//! dispatch to the loaded exporter; a later load is idempotent. The loader
//! import arrives through the SDK's own bindings; this world only imports
//! `ops`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{Digest, Plugins as _, WasiPlugins};
use omnia_test::link::ops;

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (name, declared) = match arguments.as_slice() {
        [_, name] => (name, None),
        [_, name, digest] => (name, Some(digest.parse::<Digest>().expect("a declared digest"))),
        _ => panic!("expected `<name> [digest]`; got {arguments:?}"),
    };

    // Two requests for one declared entry: whichever admits first, the
    // other attests the same registration.
    let (first, second) = futures::join!(WasiPlugins.load(name), WasiPlugins.load(name));
    let first = first.expect("the first load succeeds");
    let second = second.expect("the racing load succeeds");
    assert_eq!(first.id(), name, "a guest registers under its declared name");
    assert_eq!(second.id(), name);
    let digest = first.digest().cloned().expect("an admitted guest reports its digest");
    assert_eq!(second.digest(), Some(&digest), "both handles attest one registration");
    if let Some(declared) = &declared {
        assert_eq!(&digest, declared, "the reported digest is the declared one");
    }

    // The handle's identity routes host-mediated dispatch to the exporter.
    let answer = ops::ping(first.id(), "hi");
    assert_eq!(answer, format!("{name} pong: hi"));

    let again = WasiPlugins.load(name).await.expect("a later load attests");
    assert_eq!(again.id(), name);
    assert_eq!(again.digest(), Some(&digest));
}
