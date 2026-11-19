//! The `omnia:sql` rows, over a [`Database`].
//!
//! Each row creates tables of its own, so the rows run in any order, or all
//! at once, over one database. Every statement keeps to the portable dialect
//! the [SQL reference](https://github.com/augentic/omnia/blob/main/docs/reference/sql.md#the-portable-dialect)
//! defines. The database must start without the rows' tables.

use omnia_sdk::sql::{Database, FromColumn, Row, Statement, Transaction, Value};

use super::expect_code;

const DATE: &str = "2024-02-29";
const TIME: &str = "23:59:58";
const STAMP: &str = "2024-02-29 23:59:58";
const STAMPTZ: &str = "2024-02-29T23:59:58+00:00";

/// [`batch`](Database::batch) runs its statements in order and returns each
/// one's changed-row count.
///
/// # Panics
///
/// Panics if the batch fails, a count is wrong, or a statement did not see
/// the writes before it.
pub async fn batch(db: &Database) {
    create(db, "CREATE TABLE batch_items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)").await;

    let counts = db
        .batch(vec![
            Statement::new(
                "INSERT INTO batch_items (id, name) VALUES ($1, $2), ($3, $4)",
                &[Value::from(1_i64), Value::from("one"), Value::from(2_i64), Value::from("two")],
            ),
            Statement::new("UPDATE batch_items SET name = $1", &[Value::from("both")]),
            Statement::new("DELETE FROM batch_items WHERE id = $1", &[Value::from(2_i64)]),
        ])
        .await;

    assert_eq!(counts.expect("batch"), [2, 2, 1], "a statement did not see the ones before it");
    assert_eq!(names(db, "batch_items").await, ["both"]);
}

/// A failing statement fails the whole [`batch`](Database::batch) and rolls
/// back every statement before it.
///
/// # Panics
///
/// Panics if the batch succeeds, or a write before the failing statement
/// persists.
pub async fn batch_rollback(db: &Database) {
    create(db, "CREATE TABLE rollback_items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)").await;
    let insert = "INSERT INTO rollback_items (id, name) VALUES ($1, $2)";
    let seeded = db.exec(insert, &[Value::from(1_i64), Value::from("kept")]).await;
    assert_eq!(seeded.expect("seed"), 1);

    let batch = db
        .batch(vec![
            Statement::new(insert, &[Value::from(2_i64), Value::from("added")]),
            Statement::new(
                "UPDATE rollback_items SET name = $1 WHERE id = $2",
                &[Value::from("changed"), Value::from(1_i64)],
            ),
            Statement::new(insert, &[Value::from(1_i64), Value::from("duplicate")]),
        ])
        .await;

    assert!(batch.is_err(), "a batch with a duplicate key returned {batch:?}");
    assert_eq!(names(db, "rollback_items").await, ["kept"], "a failed batch left its writes");
}

/// A value binds to a column of another numeric type it converts to without
/// loss, and reads back unchanged.
///
/// # Panics
///
/// Panics if a value does not bind, or reads back another value.
pub async fn conversions(db: &Database) {
    create(
        db,
        "CREATE TABLE conversion_items (id BIGINT PRIMARY KEY, c_bigint BIGINT, \
         c_integer INTEGER, c_double DOUBLE PRECISION)",
    )
    .await;

    let insert = "INSERT INTO conversion_items (id, c_bigint, c_integer, c_double) \
                  VALUES ($1, $2, $3, $4)";
    let params =
        [Value::from(1_i64), Value::from(7_i32), Value::from(42_i64), Value::from(0.5_f32)];
    assert_eq!(db.exec(insert, &params).await.expect("insert"), 1);

    let rows = query(db, "SELECT c_bigint, c_integer, c_double FROM conversion_items", &[]).await;
    let [row] = &rows[..] else {
        panic!("read {} rows back, not 1", rows.len());
    };
    assert_eq!(column::<i64>(row, "c_bigint"), 7, "an `Int32` bound to `BIGINT`");
    assert_eq!(column::<i64>(row, "c_integer"), 42, "an `Int64` bound to `INTEGER`");
    assert_float(row, "c_double", 0.5);
}

/// `$n` binds the `n`th parameter, in any order and as often as it appears.
///
/// # Panics
///
/// Panics if a parameter binds to another placeholder.
pub async fn placeholders(db: &Database) {
    create(db, "CREATE TABLE placeholder_items (id BIGINT PRIMARY KEY, x TEXT, y TEXT)").await;
    let reordered = "INSERT INTO placeholder_items (y, id, x) VALUES ($3, $1, $2)";
    let inserted = db.exec(reordered, &[Value::from(1_i64), "x1".into(), "y1".into()]).await;
    assert_eq!(inserted.expect("insert with reordered placeholders"), 1);
    let repeated = "INSERT INTO placeholder_items (id, x, y) VALUES ($1, $2, $2)";
    let inserted = db.exec(repeated, &[Value::from(2_i64), "same".into()]).await;
    assert_eq!(inserted.expect("insert with a repeated placeholder"), 1);

    let rows = query(db, "SELECT x, y FROM placeholder_items ORDER BY id", &[]).await;
    let pairs: Vec<(String, String)> =
        rows.iter().map(|row| (column(row, "x"), column(row, "y"))).collect();
    let expected = [("x1", "y1"), ("same", "same")].map(|(x, y)| (x.to_owned(), y.to_owned()));
    assert_eq!(pairs, expected, "an insert bound a value to another placeholder");

    let select = "SELECT id FROM placeholder_items WHERE x = $1 OR y = $1";
    let rows = query(db, select, &["y1".into()]).await;
    let ids: Vec<i64> = rows.iter().map(|row| column(row, "id")).collect();
    assert_eq!(ids, [1], "a query bound a repeated placeholder to another value");
}

/// A [`Transaction`] commits or rolls back its statements together.
///
/// - It reads its own write, which is there after
///   [`commit`](Transaction::commit).
/// - A write is gone after [`rollback`](Transaction::rollback).
/// - A failed statement ends it, so a later call fails with the code
///   `invalid`, and nothing it wrote remains.
/// - A write is gone after the transaction drops, and its id inserts again.
///
/// Each outcome is read through `db` once the transaction has ended, since a
/// call outside a transaction may wait for it.
///
/// # Panics
///
/// Panics if a write outlives a transaction that did not commit, a committed
/// one is lost, or a call after a failure succeeds.
pub async fn transaction(db: &Database) {
    create(db, "CREATE TABLE transaction_items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)").await;
    let insert = "INSERT INTO transaction_items (id, name) VALUES ($1, $2)";
    let item = |id: i64, name: &str| [Value::from(id), Value::from(name)];

    // committed, after reading its own write
    let committed = begin(db).await;
    assert_eq!(committed.exec(insert, &item(1, "committed")).await.expect("insert"), 1);
    let select = "SELECT name FROM transaction_items WHERE id = $1";
    let rows = committed.query_as::<Row>(select, &[Value::from(1_i64)]).await.expect("query");
    let read: Vec<String> = rows.iter().map(|row| column(row, "name")).collect();
    assert_eq!(read, ["committed"], "a transaction did not read its own write");
    committed.commit().await.expect("commit");

    // rolled back
    let rolled_back = begin(db).await;
    assert_eq!(rolled_back.exec(insert, &item(2, "rolled back")).await.expect("insert"), 1);
    rolled_back.rollback().await.expect("rollback");

    // ended by a failure
    let failed = begin(db).await;
    assert_eq!(failed.exec(insert, &item(3, "failed")).await.expect("insert"), 1);
    let duplicate = failed.exec(insert, &item(1, "duplicate")).await;
    assert!(duplicate.is_err(), "a duplicate key returned {duplicate:?}");
    let after = failed.exec(insert, &item(4, "after")).await;
    expect_code(after, "invalid", "execute after a failure");
    expect_code(failed.commit().await, "invalid", "commit after a failure");

    // dropped
    let dropped = begin(db).await;
    assert_eq!(dropped.exec(insert, &item(5, "dropped")).await.expect("insert"), 1);
    drop(dropped);

    assert_eq!(names(db, "transaction_items").await, ["committed"], "only the commit persists");
    let reinserted = db.exec(insert, &item(5, "again")).await;
    assert_eq!(reinserted.expect("insert the dropped id"), 1);
}

/// Each portable column type reads back the [`Value`] it was given, as the
/// Rust type the SQL reference lists, and each `NULL` reads as `None`.
///
/// # Panics
///
/// Panics if a column reads back another value, or a `NULL` reads as one.
pub async fn values(db: &Database) {
    create(
        db,
        "CREATE TABLE value_items (id BIGINT PRIMARY KEY, c_int32 INTEGER, c_int64 BIGINT, \
         c_float REAL, c_double DOUBLE PRECISION, c_text TEXT, c_true BOOLEAN, c_false BOOLEAN, \
         c_bytes BYTEA, c_date DATE, c_time TIME, c_stamp TIMESTAMP, c_stamptz TIMESTAMPTZ)",
    )
    .await;

    let insert = "INSERT INTO value_items (id, c_int32, c_int64, c_float, c_double, c_text, \
                  c_true, c_false, c_bytes, c_date, c_time, c_stamp, c_stamptz) \
                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)";
    let given = [
        Value::from(1_i64),
        Value::from(i32::MAX),
        Value::from(i64::MIN),
        Value::from(1.5_f32),
        Value::from(-2.25_f64),
        Value::from("héllo"),
        Value::from(true),
        Value::from(false),
        Value::from(vec![0_u8, 1, 254, 255]),
        Value::Date(DATE.to_owned()),
        Value::Time(TIME.to_owned()),
        Value::Timestamp(STAMP.to_owned()),
        Value::Timestamp(STAMPTZ.to_owned()),
    ];
    let mut null = [const { Value::Null }; 13];
    null[0] = Value::from(2_i64);
    for params in [given, null] {
        assert_eq!(db.exec(insert, &params).await.expect("insert"), 1);
    }

    let rows = query(db, "SELECT * FROM value_items ORDER BY id", &[]).await;
    let [given, null] = &rows[..] else {
        panic!("read {} rows back, not 2", rows.len());
    };

    assert_eq!(column::<i32>(given, "c_int32"), i32::MAX, "c_int32");
    assert_eq!(column::<i64>(given, "c_int64"), i64::MIN, "c_int64");
    assert_float(given, "c_float", 1.5);
    assert_float(given, "c_double", -2.25);
    assert_eq!(column::<String>(given, "c_text"), "héllo", "c_text");
    assert!(column::<bool>(given, "c_true"), "c_true");
    assert!(!column::<bool>(given, "c_false"), "c_false");
    assert_eq!(column::<Vec<u8>>(given, "c_bytes"), [0, 1, 254, 255], "c_bytes");
    assert_eq!(column::<String>(given, "c_date"), DATE, "c_date");
    assert_eq!(column::<String>(given, "c_time"), TIME, "c_time");
    assert_eq!(column::<String>(given, "c_stamp"), STAMP, "c_stamp");
    assert_eq!(column::<String>(given, "c_stamptz"), STAMPTZ, "c_stamptz");

    for name in [
        "c_int32",
        "c_int64",
        "c_float",
        "c_double",
        "c_text",
        "c_true",
        "c_false",
        "c_bytes",
        "c_date",
        "c_time",
        "c_stamp",
        "c_stamptz",
    ] {
        let value = &null.fields.iter().find(|field| field.name == name).expect(name).value;
        assert!(value.is_null(), "`{name}` read back {value:?}, not `NULL`");
    }
}

async fn create(db: &Database, ddl: &str) {
    db.exec(ddl, &[]).await.unwrap_or_else(|err| panic!("{ddl}: {err}"));
}

async fn begin(db: &Database) -> Transaction {
    db.begin().await.unwrap_or_else(|err| panic!("begin: {err}"))
}

async fn query(db: &Database, sql: &str, params: &[Value]) -> Vec<Row> {
    db.query_as(sql, params).await.unwrap_or_else(|err| panic!("{sql}: {err}"))
}

async fn names(db: &Database, table: &str) -> Vec<String> {
    let rows = query(db, &format!("SELECT name FROM {table} ORDER BY id"), &[]).await;
    rows.iter().map(|row| column(row, "name")).collect()
}

#[track_caller]
fn column<'a, T: FromColumn<'a>>(row: &'a Row, name: &str) -> T {
    row.get(name).unwrap_or_else(|err| panic!("read `{name}`: {err}"))
}

#[track_caller]
fn assert_float(row: &Row, name: &str, expected: f64) {
    let read: f64 = column(row, name);
    assert!((read - expected).abs() < f64::EPSILON, "`{name}` read back {read}, not {expected}");
}
