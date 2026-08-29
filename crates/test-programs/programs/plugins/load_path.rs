//! Happy path: load a mounted component unpinned (trust-on-first-use — the
//! resolved digest returns), dispatch through the returned handle's identity,
//! then prove idempotency by re-loading pinned with that digest.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "requester",
    path: "wit",
    generate_all,
});

use omnia::plugins::loader::{self, Location};
use omnia_test::link::ops;

test_programs::run!(scenario);

async fn scenario() {
    let plugin =
        loader::load("test:echoer".to_owned(), Location::Path("./plugin.wasm".to_owned()), None)
            .await
            .expect("unpinned load succeeds");
    assert_eq!(plugin.id(), "test:echoer");
    let digest = plugin.digest();
    assert!(
        digest.starts_with("sha256:") && digest.len() == "sha256:".len() + 64,
        "the resolved digest is a sha256 pin: {digest}"
    );

    // The handle's identity routes host-mediated dispatch to the exporter.
    let answer = ops::ping(&plugin.id(), "hi");
    assert_eq!(answer, "test:echoer pong: hi");

    // Second load is idempotent, and the reported digest pins it exactly.
    let again = loader::load(
        "test:echoer".to_owned(),
        Location::Path("./plugin.wasm".to_owned()),
        Some(digest.clone()),
    )
    .await
    .expect("pinned re-load succeeds");
    assert_eq!(again.id(), plugin.id());
    assert_eq!(again.digest(), digest);
}
