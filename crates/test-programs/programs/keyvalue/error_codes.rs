//! Every `omnia:keyvalue` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! host rejects a zero TTL and an undeclared bucket itself; the suite's
//! faulting backend fails the rest by bucket name.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;
use std::time::Duration;

use omnia_sdk::Result;
use omnia_sdk::keyvalue::{DEFAULT, store};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // the host's own rules
    let default = store::open(DEFAULT.to_owned()).await.expect("open the default bucket");
    expect_code(default.set_json("key", "v", Some(Duration::ZERO)).await, 500, "invalid");
    expect_code(get("undeclared").await, 500, "not-found");

    // the backend's chosen cases
    for (bucket, status, code) in [
        ("denied", 500, "denied"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(get(bucket).await, status, code);
    }
}

async fn get(bucket: &str) -> Result<Option<String>> {
    store::open(bucket.to_owned()).await?.get_json("key").await
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
