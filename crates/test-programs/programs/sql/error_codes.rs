//! Every `omnia:sql` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! default backend fails an undeclared database and a malformed statement
//! itself. The suite's faulting backend fails the rest by database name. The
//! `wasi:sql` draft's error carries only the message.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::{Result, SqlDb as _, WasiProvider};
use omnia_wasi_sql::types::Connection;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the default backend's own cases
    expect_code(provider.named("undeclared").query("SELECT 1", &[]).await, 500, "not-found");
    expect_code(provider.execute("SELEKT nonsense", &[]).await, 500, "invalid");

    // the backend's chosen cases
    for (database, status, code) in [
        ("denied", 500, "denied"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(provider.named(database).query("SELECT 1", &[]).await, status, code);
    }

    // the draft's error has no case
    let Err(error) = Connection::open("denied".to_owned()).await else {
        panic!("the draft opened a denied database");
    };
    assert_eq!(error.trace(), "denied by the suite");
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
