//! Every `omnia:config` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest.
//!
//! - The suite's faulting backend fails `get` by key.
//! - It lists nothing, so a listing answers `unsupported`.
//! - The host refuses a zero `limit` as `invalid` before the backend is asked.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::Result;
use omnia_sdk::config::{self, entries, types};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // the backend's chosen cases
    for (key, status, code) in [
        ("NOT_FOUND", 500, "not-found"),
        ("DENIED", 500, "denied"),
        ("INVALID", 500, "invalid"),
        ("UNSUPPORTED", 500, "unsupported"),
        ("UNAVAILABLE", 502, "unavailable"),
        ("OTHER", 500, "other"),
    ] {
        expect_code(config::parse::<String>(key).await, status, code);
    }

    // a listing the backend does not serve
    let listing = entries::list(String::new(), None, None).await;
    assert!(matches!(listing, Err(types::Error::Unsupported(_))), "{listing:?}");

    // the host's refusal
    let listing = entries::list(String::new(), None, Some(0)).await;
    assert!(matches!(listing, Err(types::Error::Invalid(_))), "{listing:?}");
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
