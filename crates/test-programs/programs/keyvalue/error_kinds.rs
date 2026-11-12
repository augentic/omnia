//! Every `omnia:keyvalue` error kind reaches the guest as its own
//! `ErrorKind`. The host rejects a zero TTL and an undeclared bucket itself;
//! the suite's faulting backend fails the rest by bucket name. The
//! `wasi:keyvalue` draft names only two of them, `no-such-store` and
//! `access-denied`.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;
use std::time::Duration;

use omnia_sdk::capability::{ErrorKind, Result};
use omnia_sdk::{StateStore as _, WasiProvider};
use omnia_wasi_keyvalue::store;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the host's own rules
    expect_kind(provider.set("key", b"v", Some(Duration::ZERO)).await, ErrorKind::Invalid);
    expect_kind(provider.named("undeclared").get("key").await, ErrorKind::NotFound);

    // the backend's chosen kinds
    for (bucket, kind) in [
        ("denied", ErrorKind::Denied),
        ("unsupported", ErrorKind::Unsupported),
        ("unavailable", ErrorKind::Unavailable),
        ("other", ErrorKind::Other),
    ] {
        expect_kind(provider.named(bucket).get("key").await, kind);
    }

    // the draft's two named kinds
    let undeclared = store::open("undeclared".to_owned()).await.map(drop);
    let denied = store::open("denied".to_owned()).await.map(drop);
    assert!(matches!(undeclared, Err(store::Error::NoSuchStore)), "{undeclared:?}");
    assert!(matches!(denied, Err(store::Error::AccessDenied)), "{denied:?}");
}

fn expect_kind<T: Debug>(result: Result<T>, kind: ErrorKind) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{kind}` failure"),
        Err(err) => assert_eq!(err.kind(), kind, "{err}"),
    }
}
