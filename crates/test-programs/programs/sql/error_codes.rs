//! Every `omnia:sql` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest.
//!
//! - The default backend fails an undeclared database and a malformed
//!   statement itself.
//! - The host refuses a statement that starts a transaction, a temporal value
//!   outside its format, and a call on an ended transaction, each as
//!   `invalid`.
//! - The suite's faulting backend fails the rest by database name, and serves
//!   `untransacted` with no transactions.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::sql::{Transaction as _, Value};
use omnia_sdk::{Result, SqlDb as _, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the default backend's own cases
    expect_code(provider.named("undeclared").query("SELECT 1", &[]).await, 500, "not-found");
    expect_code(provider.execute("SELEKT nonsense", &[]).await, 500, "invalid");

    // the host's refusals
    expect_code(provider.execute("BEGIN", &[]).await, 500, "invalid");
    let date = [Value::Date("29/02/2024".to_owned())];
    expect_code(provider.query("SELECT $1", &date).await, 500, "invalid");
    let transaction = provider.begin().await.expect("begin");
    expect_code(transaction.execute("SELEKT nonsense", &[]).await, 500, "invalid");
    expect_code(transaction.execute("SELECT 1", &[]).await, 500, "invalid");
    drop(transaction);

    // the backend's chosen cases
    for (database, status, code) in [
        ("denied", 500, "denied"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(provider.named(database).query("SELECT 1", &[]).await, status, code);
    }
    expect_code(provider.named("untransacted").begin().await, 500, "unsupported");
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
