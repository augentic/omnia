//! Every case a registry source reports reaches an SDK requester as an
//! `omnia_sdk::Error` under the case's name, and a missed pin as the
//! conflict the load answers with, naming the bytes found. Argv carries the
//! echoer's digest.
//!
//! - The suite's faulting source fails `fault:<case>@1.0.0` with that case.
//! - It serves the echoer as `test:echoer@1.0.0`, which the pin here misses.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{self, Release, Source};
use test_programs::fails_with;

omnia_sdk::command::export!(scenario);

const MISSED: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, echoer] = arguments.as_slice() else {
        panic!("expected `<echoer digest>`; got {arguments:?}");
    };

    // the source's chosen cases
    for case in ["not-found", "denied", "invalid", "unsupported", "unavailable"] {
        let source = release(&format!("fault:{case}@1.0.0"), None);
        let error = plugins::load(source).await.expect_err("the source fails the load");
        fails_with(&error, case, case);
    }

    // a release whose bytes miss the pin
    let loaded = plugins::load(release("test:echoer@1.0.0", Some(MISSED))).await;
    let conflict = loaded.expect("the load runs").expect_err("the bytes miss the pin");
    assert_eq!(&conflict.found, echoer, "the conflict names the bytes found");
    assert!(conflict.current.is_none(), "no guest is loaded under the name");
}

fn release(reference: &str, pin: Option<&str>) -> Source {
    Source::Release(Release {
        reference: reference.to_owned(),
        registry: None,
        pin: pin.map(str::to_owned),
    })
}
