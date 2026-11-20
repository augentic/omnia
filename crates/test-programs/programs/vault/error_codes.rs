//! Every `omnia:vault` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. An
//! undeclared locker fails at `open`, and the suite's faulting backend fails
//! `get` by locker name.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::Result;
use omnia_sdk::vault::store;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    for (name, status, code) in [
        ("undeclared", 500, "not-found"),
        ("denied", 500, "denied"),
        ("invalid", 500, "invalid"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(get(name).await, status, code);
    }
}

async fn get(locker: &str) -> Result<Option<Vec<u8>>> {
    Ok(store::open(locker.to_owned()).await?.get("token".to_owned()).await?)
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
