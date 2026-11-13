//! The [`SqlDb`] rows.
//!
//! Each row creates tables of its own, so the rows run in any order, or all
//! at once, over one database. Every statement keeps to the portable dialect
//! the [SQL reference](https://github.com/augentic/omnia/blob/main/docs/reference/sql.md#the-portable-dialect)
//! defines. The database must start without the rows' tables.

use crate::SqlDb;
use crate::sql::{FromColumn, Row, Statement, Value};

const DATE: &str = "2024-02-29";
const TIME: &str = "23:59:58";
const STAMP: &str = "2024-02-29 23:59:58";
const STAMPTZ: &str = "2024-02-29T23:59:58+00:00";

/// [`batch`](SqlDb::batch) runs its statements in order and returns each
/// one's changed-row count.
///
/// # Panics
///
/// Panics if the batch fails, a count is wrong, or a statement did not see
/// the writes before it.
pub async fn batch(db: &impl SqlDb) {
    create(db, "CREATE TABLE batch_items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)").await;

    let counts = db
        .batch(&[
            Statement::new(
                "INSERT INTO batch_items (id, name) VALUES ($1, $2), ($3, $4)",
                [Value::from(1_i64), Value::from("one"), Value::from(2_i64), Value::from("two")],
            ),
            Statement::new("UPDATE batch_items SET name = $1", [Value::from("both")]),
            Statement::new("DELETE FROM batch_items WHERE id = $1", [Value::from(2_i64)]),
        ])
        .await;

    assert_eq!(counts.expect("batch"), [2, 2, 1], "a statement did not see the ones before it");
    assert_eq!(names(db, "batch_items").await, ["both"]);
}

/// A failing statement fails the whole [`batch`](SqlDb::batch) and rolls
/// back every statement before it.
///
/// # Panics
///
/// Panics if the batch succeeds, or a write before the failing statement
/// persists.
pub async fn batch_rollback(db: &impl SqlDb) {
    create(db, "CREATE TABLE rollback_items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)").await;
    let insert = "INSERT INTO rollback_items (id, name) VALUES ($1, $2)";
    let seeded = db.execute(insert, &[Value::from(1_i64), Value::from("kept")]).await;
    assert_eq!(seeded.expect("seed"), 1);

    let batch = db
        .batch(&[
            Statement::new(insert, [Value::from(2_i64), Value::from("added")]),
            Statement::new(
                "UPDATE rollback_items SET name = $1 WHERE id = $2",
                [Value::from("changed"), Value::from(1_i64)],
            ),
            Statement::new(insert, [Value::from(1_i64), Value::from("duplicate")]),
        ])
        .await;

    assert!(batch.is_err(), "a batch with a duplicate key returned {batch:?}");
    assert_eq!(names(db, "rollback_items").await, ["kept"], "a failed batch left its writes");
}

/// `$n` binds the `n`th parameter, in any order and as often as it appears.
///
/// # Panics
///
/// Panics if a parameter binds to another placeholder.
pub async fn placeholders(db: &impl SqlDb) {
    create(db, "CREATE TABLE placeholder_items (id BIGINT PRIMARY KEY, x TEXT, y TEXT)").await;
    let reordered = "INSERT INTO placeholder_items (y, id, x) VALUES ($3, $1, $2)";
    let inserted = db.execute(reordered, &[Value::from(1_i64), "x1".into(), "y1".into()]).await;
    assert_eq!(inserted.expect("insert with reordered placeholders"), 1);
    let repeated = "INSERT INTO placeholder_items (id, x, y) VALUES ($1, $2, $2)";
    let inserted = db.execute(repeated, &[Value::from(2_i64), "same".into()]).await;
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

/// Each portable column type reads back the [`Value`] it was given, as the
/// Rust type the SQL reference lists, and each `NULL` reads as `None`.
///
/// # Panics
///
/// Panics if a column reads back another value, or a `NULL` reads as one.
pub async fn values(db: &impl SqlDb) {
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
        Value::Date(Some(DATE.to_owned())),
        Value::Time(Some(TIME.to_owned())),
        Value::Timestamp(Some(STAMP.to_owned())),
        Value::Timestamp(Some(STAMPTZ.to_owned())),
    ];
    let null = [
        Value::from(2_i64),
        Value::Int32(None),
        Value::Int64(None),
        Value::Float(None),
        Value::Double(None),
        Value::Str(None),
        Value::Boolean(None),
        Value::Boolean(None),
        Value::Binary(None),
        Value::Date(None),
        Value::Time(None),
        Value::Timestamp(None),
        Value::Timestamp(None),
    ];
    for params in [given, null] {
        assert_eq!(db.execute(insert, &params).await.expect("insert"), 1);
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

async fn create(db: &impl SqlDb, ddl: &str) {
    db.execute(ddl, &[]).await.unwrap_or_else(|err| panic!("{ddl}: {err}"));
}

async fn query(db: &impl SqlDb, sql: &str, params: &[Value]) -> Vec<Row> {
    db.query(sql, params).await.unwrap_or_else(|err| panic!("{sql}: {err}"))
}

async fn names(db: &impl SqlDb, table: &str) -> Vec<String> {
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
