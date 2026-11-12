//! Named databases through the `omnia_sdk` `SqlDb`: the unnamed handle and
//! `second` are separate databases, `execute` counts the rows a statement
//! touched, an undeclared name fails on first use, and `batch` is
//! unsupported over `wasi:sql`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::capability::ErrorKind;
use omnia_sdk::sql::{Statement, Value};
use omnia_sdk::{SqlDb, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let second = WasiProvider.named("second");

    // the same table in each database holds its own rows
    let create = "CREATE TABLE items (name TEXT)";
    assert_eq!(WasiProvider.execute(create, &[]).await.expect("create in default"), 0);
    assert_eq!(second.execute(create, &[]).await.expect("create in second"), 0);
    let insert = "INSERT INTO items (name) VALUES (?), (?)";
    let inserted = WasiProvider.execute(insert, &[Value::from("alpha"), Value::from("beta")]).await;
    assert_eq!(inserted.expect("insert into default"), 2);
    let insert = "INSERT INTO items (name) VALUES (?)";
    let inserted = second.execute(insert, &[Value::from("gamma")]).await;
    assert_eq!(inserted.expect("insert into second"), 1);
    assert_eq!(names(&WasiProvider).await, ["alpha", "beta"]);
    assert_eq!(names(&WasiProvider.named("default")).await, ["alpha", "beta"]);
    assert_eq!(names(&second).await, ["gamma"]);

    // an undeclared name fails on first use
    let undeclared = WasiProvider.named("undeclared");
    let error = undeclared.query("SELECT 1", &[]).await.expect_err("undeclared database");
    assert!(error.to_string().contains("not declared"), "unexpected error: {error}");

    // `wasi:sql` has no transaction to run a batch in, so nothing is written
    let batch = [Statement::new(insert, [Value::from("delta")])];
    let error = WasiProvider.batch(&batch).await.expect_err("batch");
    assert_eq!(error.kind(), ErrorKind::Unsupported, "unexpected error: {error}");
    assert_eq!(names(&WasiProvider).await, ["alpha", "beta"]);
}

async fn names(db: &impl SqlDb) -> Vec<String> {
    let rows = db.query("SELECT name FROM items ORDER BY name", &[]).await.expect("select");
    rows.iter().map(|row| row.get("name").expect("a text column")).collect()
}
