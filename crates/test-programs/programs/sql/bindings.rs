//! Named databases through the `omnia_sdk` `Database` helpers.
//!
//! - `default` and `second` are separate databases, and `exec` counts the
//!   rows a statement touched.
//! - A transaction on `second` commits there alone.
//! - A `u32` binds as a signed 64-bit integer, and an `i64::MAX` column reads
//!   back as a `u64`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::sql::database::Database;
use omnia_sdk::sql::{DEFAULT, Row, Value, store, transactions};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let default = store::open(DEFAULT.to_owned()).await.expect("open default");
    let second = store::open("second".to_owned()).await.expect("open second");

    // the same table in each database holds its own rows
    let create = "CREATE TABLE items (name TEXT)";
    assert_eq!(default.exec(create, &[]).await.expect("create in default"), 0);
    assert_eq!(second.exec(create, &[]).await.expect("create in second"), 0);
    let insert = "INSERT INTO items (name) VALUES ($1), ($2)";
    let inserted = default.exec(insert, &[Value::from("alpha"), Value::from("beta")]).await;
    assert_eq!(inserted.expect("insert into default"), 2);
    let insert = "INSERT INTO items (name) VALUES ($1)";
    let inserted = second.exec(insert, &[Value::from("gamma")]).await;
    assert_eq!(inserted.expect("insert into second"), 1);
    assert_eq!(names(&default).await, ["alpha", "beta"]);
    assert_eq!(names(&second).await, ["gamma"]);

    // a transaction on `second` commits there alone
    let transaction = transactions::begin(&second).await.expect("begin on second");
    let inserted = transaction.exec(insert, &[Value::from("delta")]).await;
    assert_eq!(inserted.expect("insert in a transaction"), 1);
    transaction.commit().await.expect("commit");
    assert_eq!(names(&second).await, ["delta", "gamma"]);
    assert_eq!(names(&default).await, ["alpha", "beta"]);

    // unsigned values travel as signed 64-bit ones
    let create = "CREATE TABLE counts (c_u32 BIGINT, c_u64 BIGINT)";
    assert_eq!(default.exec(create, &[]).await.expect("create counts"), 0);
    let widest = i64::MAX.cast_unsigned();
    let insert = "INSERT INTO counts (c_u32, c_u64) VALUES ($1, $2)";
    let inserted = default.exec(insert, &[Value::from(u32::MAX), Value::from(i64::MAX)]).await;
    assert_eq!(inserted.expect("insert counts"), 1);
    let rows: Vec<Row> =
        default.query_as("SELECT c_u32, c_u64 FROM counts", &[]).await.expect("select");
    let [row] = &rows[..] else {
        panic!("read {} rows back, not 1", rows.len());
    };
    assert_eq!(row.get::<u32>("c_u32").expect("a u32"), u32::MAX);
    assert_eq!(row.get::<u64>("c_u64").expect("a u64"), widest);
}

async fn names(db: &Database) -> Vec<String> {
    let rows: Vec<Row> =
        db.query_as("SELECT name FROM items ORDER BY name", &[]).await.expect("select");
    rows.iter().map(|row| row.get("name").expect("a text column")).collect()
}
