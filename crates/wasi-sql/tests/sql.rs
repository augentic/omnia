//! End-to-end tests for `wasi:sql`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory `SQLite` default. The guest asserts what it observes across
//! the boundary (and traps on failure); the host side asserts what persisted
//! in the shared database after the run.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_sql::{DataType, WasiSql, WasiSqlCtx as _};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_sql!();

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiSql, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn sql_query_exec() {
    let backends = Backends::defaults().await;

    run_guest(test_programs::SQL_QUERY_EXEC, backends.clone()).await;

    // Persisted state: the same `:memory:` database the guest wrote holds the
    // one row its delete left behind.
    let db = backends.sql.open("db".to_owned()).await.expect("open");
    let rows = db.query("SELECT name FROM items".to_owned(), vec![]).await.expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields.len(), 1);
    assert_eq!(rows[0].fields[0].name, "name");
    let DataType::Str(Some(name)) = &rows[0].fields[0].value else {
        panic!("unexpected value: {:?}", rows[0].fields[0].value);
    };
    assert_eq!(name, "alpha");
}
