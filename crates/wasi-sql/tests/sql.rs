//! End-to-end tests for `wasi:sql`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory `SQLite` default. The guest asserts what it observes across
//! the boundary (and traps on failure); the host side asserts what persisted
//! in the shared database after the run.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;

use omnia::{Connect as _, ExitStatus};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_sql::{ConnectOptions, DataType, SqlClient as _, SqlDefault, WasiSql};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_sql!();

async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiSql, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

async fn names(sql: &SqlDefault, database: &str) -> Vec<String> {
    let db = sql.open(database.to_owned()).await.expect("open");
    let rows = db.query("SELECT name FROM items ORDER BY name".to_owned(), vec![]).await;
    rows.expect("query")
        .into_iter()
        .map(|row| {
            let [field] = &row.fields[..] else {
                panic!("unexpected fields: {:?}", row.fields);
            };
            let DataType::Str(Some(name)) = &field.value else {
                panic!("unexpected value: {:?}", field.value);
            };
            assert_eq!(field.name, "name");
            name.clone()
        })
        .collect()
}

#[tokio::test]
async fn sql_query_exec() {
    let backends = Backends::defaults().await;

    run_guest(test_programs::SQL_QUERY_EXEC, backends.clone()).await;

    // Persisted state: the same `:memory:` database the guest wrote holds the
    // one row its delete left behind, under the name its update gave it.
    assert_eq!(names(&backends.sql, "default").await, ["renamed"]);
}

#[tokio::test]
async fn sql_bindings() {
    let options = ConnectOptions {
        database: ":memory:".to_owned(),
        databases: BTreeMap::from([("second".to_owned(), ":memory:".to_owned())]),
    };
    let sql = SqlDefault::connect_with(options).await.expect("in-memory sqlite");
    let backends = Backends::defaults().await.sql(sql.clone());

    run_guest(test_programs::SQL_BINDINGS, backends).await;

    // Persisted state: the declared name is its own database, which holds
    // only what the guest wrote through it.
    assert_eq!(names(&sql, "second").await, ["gamma"]);
}
