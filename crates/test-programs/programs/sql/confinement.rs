//! The `SQLite` default holds a guest's SQL to the database it opened.
//!
//! - A statement that reaches a host file is `denied`, whether it names the
//!   file as a literal or binds it.
//! - A plain `VACUUM` is `denied` too, since it attaches a database.
//! - After `PRAGMA writable_schema = ON`, a write to `sqlite_schema` is still
//!   `invalid`.
//!
//! The suite passes a host directory as the first argument, and checks that
//! it stays empty.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::Result;
use omnia_sdk::sql::{self, Value};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let dir = std::env::args().nth(1).expect("a host directory");
    let bound = |file: &str| vec![Value::Text(format!("{dir}/{file}"))];

    // statements that reach a host file
    let reaching = [
        (format!("ATTACH '{dir}/literal.db' AS outside"), vec![]),
        ("ATTACH $1 AS outside".to_owned(), bound("bound.db")),
        (format!("VACUUM INTO '{dir}/literal-copy.db'"), vec![]),
        ("VACUUM INTO $1".to_owned(), bound("bound-copy.db")),
        ("VACUUM".to_owned(), vec![]),
        (format!("PRAGMA temp_store_directory = '{dir}'"), vec![]),
    ];
    for (statement, params) in reaching {
        expect_code(exec(&statement, &params).await, "denied", &statement);
    }

    // a schema ordinary SQL cannot rewrite
    exec("PRAGMA writable_schema = ON", &[]).await.expect("the pragma runs");
    let rewrite = "UPDATE sqlite_schema SET sql = sql";
    expect_code(exec(rewrite, &[]).await, "invalid", rewrite);
}

async fn exec(statement: &str, params: &[Value]) -> Result<u64> {
    sql::default().await?.exec(statement, params).await
}

fn expect_code(result: Result<u64>, code: &str, statement: &str) {
    match result {
        Ok(changed) => panic!("`{statement}` changed {changed} rows, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (500, code), "`{statement}`: {err}");
        }
    }
}
