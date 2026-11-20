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

use omnia_sdk::Result;
use omnia_sdk::sql::transactions::{self, Transaction};
use omnia_sdk::sql::{DEFAULT, Row, Value, store};

omnia_sdk::command::export!(scenario);

// each call opens its database, so the suite sees one open per call
async fn scenario() {
    // the default backend's own cases
    expect_code(query("undeclared", "SELECT 1", &[]).await, 500, "not-found");
    expect_code(exec(DEFAULT, "SELEKT nonsense").await, 500, "invalid");

    // the host's refusals
    expect_code(exec(DEFAULT, "BEGIN").await, 500, "invalid");
    let date = [Value::Date("29/02/2024".to_owned())];
    expect_code(query(DEFAULT, "SELECT $1", &date).await, 500, "invalid");
    let transaction = begin(DEFAULT).await.expect("begin");
    expect_code(transaction.exec("SELEKT nonsense", &[]).await, 500, "invalid");
    expect_code(transaction.exec("SELECT 1", &[]).await, 500, "invalid");
    drop(transaction);

    // the backend's chosen cases
    for (database, status, code) in [
        ("denied", 500, "denied"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(query(database, "SELECT 1", &[]).await, status, code);
    }
    expect_code(begin("untransacted").await, 500, "unsupported");
}

async fn query(database: &str, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
    store::open(database.to_owned()).await?.query_as(sql, params).await
}

async fn exec(database: &str, sql: &str) -> Result<u64> {
    store::open(database.to_owned()).await?.exec(sql, &[]).await
}

async fn begin(database: &str) -> Result<Transaction> {
    let database = store::open(database.to_owned()).await?;
    Ok(transactions::begin(&database).await?)
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
