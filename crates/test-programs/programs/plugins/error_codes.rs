//! Every case a registry source reports reaches an SDK requester as the code
//! it folds onto, and a conflict as a refusal naming the bytes found. Argv
//! carries the echoer's digest.
//!
//! - The suite's faulting source fails `fault:<case>@1.0.0` with that case.
//! - It serves the echoer as `test:echoer@1.0.0`, which the pin here misses.
//! - A declared name with a pin is refused before it reaches the host.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::WasiProvider;
use omnia_sdk::plugins::{Digest, Error, Location, Plugins as _};

omnia_sdk::api::command::export!(scenario);

const MISSED: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, echoer] = arguments.as_slice() else {
        panic!("expected `<echoer digest>`; got {arguments:?}");
    };

    // the source's chosen cases
    for (case, status, code) in [
        ("not-found", 400, "refused"),
        ("denied", 400, "refused"),
        ("invalid", 400, "refused"),
        ("unsupported", 500, "internal"),
        ("unavailable", 502, "unavailable"),
    ] {
        let from = release(&format!("fault:{case}@1.0.0"));
        expect_code(WasiProvider.load(&from, None).await, (status, code), case);
    }

    // a release whose bytes miss the pin
    let pin: Digest = MISSED.parse().expect("a well-formed digest");
    let conflict = WasiProvider.load(&release("test:echoer@1.0.0"), Some(&pin)).await;
    expect_code(conflict, (400, "refused"), echoer);

    // a declared name the call pins, which the SDK refuses unsent
    let declared = WasiProvider.load(&Location::Declared("echoer".to_owned()), Some(&pin)).await;
    expect_code(declared, (400, "refused"), "takes no digest");
}

fn release(reference: &str) -> Location {
    Location::Registry {
        package: reference.to_owned(),
        endpoint: None,
    }
}

fn expect_code<T: Debug>(result: Result<T, Error>, expected: (u16, &str), needle: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{}` failure", expected.1),
        Err(err) => {
            let err = omnia_sdk::Error::from(err);
            assert_eq!((err.status().as_u16(), err.code()), expected, "{err}");
            assert!(err.description().contains(needle), "`{err}` does not mention `{needle}`");
        }
    }
}
