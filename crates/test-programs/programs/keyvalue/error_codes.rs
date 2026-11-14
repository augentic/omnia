//! Every `omnia:keyvalue` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! host rejects a zero TTL and an undeclared bucket itself; the suite's
//! faulting backend fails the rest by bucket name. The `wasi:keyvalue` draft
//! names only two of them, `no-such-store` and `access-denied`.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;
use std::time::Duration;

use omnia_sdk::{Result, StateStore as _, WasiProvider};
use omnia_wasi_keyvalue::store;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the host's own rules
    expect_code(provider.set("key", b"v", Some(Duration::ZERO)).await, 500, "invalid");
    expect_code(provider.named("undeclared").get("key").await, 500, "not-found");

    // the backend's chosen cases
    for (bucket, status, code) in [
        ("denied", 500, "denied"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(provider.named(bucket).get("key").await, status, code);
    }

    // the draft's two named cases
    let undeclared = store::open("undeclared".to_owned()).await.map(drop);
    let denied = store::open("denied".to_owned()).await.map(drop);
    assert!(matches!(undeclared, Err(store::Error::NoSuchStore)), "{undeclared:?}");
    assert!(matches!(denied, Err(store::Error::AccessDenied)), "{denied:?}");
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
