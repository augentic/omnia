//! One database walked end to end over the raw `omnia:sql` bindings, against
//! the `SQLite` default.
//!
//! - DDL changes no row, so it returns zero, even right after an insert.
//! - Every `value` case binds, and reads back in the `SQLite` storage class
//!   it was stored in. `none` binds `NULL`, which reads back as `none`.
//! - `columns` names the select list even when no row matches, and `rows`
//!   streams every row before `finished` resolves `Ok`.
//! - A temporal value outside its case's format is `invalid`, naming its
//!   parameter.
//! - Reading `TEXT` that is not valid UTF-8 is `other`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::sql::database::Database;
use omnia_sdk::sql::store;
use omnia_sdk::sql::types::{Error, ResultSet, Row, Value};

omnia_sdk::command::export!(scenario);

const DATE: &str = "2024-02-29";
const TIME: &str = "23:59:58.5";
const STAMP: &str = "2024-02-29 23:59:58";
const STAMPTZ: &str = "2024-02-29T23:59:58+00:00";
const COLUMNS: [&str; 12] = [
    "id",
    "c_boolean",
    "c_int32",
    "c_int64",
    "c_float32",
    "c_float64",
    "c_text",
    "c_binary",
    "c_date",
    "c_time",
    "c_stamp",
    "c_stamptz",
];

async fn scenario() {
    let database = store::open("default".to_owned()).await.expect("open");

    // DDL changes no row, even right after an insert
    let create = "CREATE TABLE items (id BIGINT PRIMARY KEY, c_boolean BOOLEAN, \
                  c_int32 INTEGER, c_int64 BIGINT, c_float32 REAL, c_float64 DOUBLE PRECISION, \
                  c_text TEXT, c_binary BYTEA, c_date DATE, c_time TIME, c_stamp TIMESTAMP, \
                  c_stamptz TIMESTAMPTZ)";
    assert_eq!(database.execute(create.to_owned(), vec![]).await.expect("create"), 0);
    let insert = "INSERT INTO items VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";
    let every_case = vec![
        Some(Value::Int64(1)),
        Some(Value::Boolean(true)),
        Some(Value::Int32(-7)),
        Some(Value::Int64(i64::MAX)),
        Some(Value::Float32(1.5)),
        Some(Value::Float64(-2.25)),
        Some(Value::Text("héllo".to_owned())),
        Some(Value::Binary(vec![0, 1, 254, 255])),
        Some(Value::Date(DATE.to_owned())),
        Some(Value::Time(TIME.to_owned())),
        Some(Value::Timestamp(STAMP.to_owned())),
        Some(Value::TimestampTz(STAMPTZ.to_owned())),
    ];
    assert_eq!(database.execute(insert.to_owned(), every_case).await.expect("insert"), 1);
    let mut nulls = vec![None; COLUMNS.len()];
    nulls[0] = Some(Value::Int64(2));
    assert_eq!(database.execute(insert.to_owned(), nulls).await.expect("insert NULLs"), 1);
    let index = "CREATE INDEX items_text ON items (c_text)";
    assert_eq!(database.execute(index.to_owned(), vec![]).await.expect("create index"), 0);

    // each case reads back in the storage class it was stored in
    let (columns, rows) = select(&database, "SELECT * FROM items ORDER BY id", vec![]).await;
    assert_eq!(columns, COLUMNS);
    let [given, nulls] = &rows[..] else {
        panic!("read {} rows back, not 2", rows.len());
    };
    let stored = [
        Some(Value::Int64(1)),
        Some(Value::Int64(1)),
        Some(Value::Int64(-7)),
        Some(Value::Int64(i64::MAX)),
        Some(Value::Float64(1.5)),
        Some(Value::Float64(-2.25)),
        Some(Value::Text("héllo".to_owned())),
        Some(Value::Binary(vec![0, 1, 254, 255])),
        Some(Value::Text(DATE.to_owned())),
        Some(Value::Text(TIME.to_owned())),
        Some(Value::Text(STAMP.to_owned())),
        Some(Value::Text(STAMPTZ.to_owned())),
    ];
    assert_same(given, &stored);
    assert!(nulls[1..].iter().all(Option::is_none), "a `NULL` read back as {nulls:?}");

    // a select that matches no row still names its columns
    let none = "SELECT id, c_text AS label FROM items WHERE id > $1";
    let (columns, rows) = select(&database, none, vec![Some(Value::Int64(2))]).await;
    assert_eq!(columns, ["id", "label"]);
    assert!(rows.is_empty(), "selected {rows:?}");

    // `rows` streams a long select to its end
    let counted = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 500) \
                   SELECT i FROM n";
    let (_, rows) = select(&database, counted, vec![]).await;
    let read: Vec<i64> = rows
        .iter()
        .map(|row| match &row[..] {
            [Some(Value::Int64(i))] => *i,
            other => panic!("read {other:?}"),
        })
        .collect();
    assert_eq!(read, (1..=500).collect::<Vec<_>>());

    // a temporal value outside its case's format does not bind
    for value in [
        Value::Date("2024-2-29".to_owned()),
        Value::Time("24:00:00".to_owned()),
        Value::Timestamp("2024-02-29T23:59:58".to_owned()),
        Value::TimestampTz(STAMP.to_owned()),
        Value::TimestampTz("2024-02-29T23:59:58+00".to_owned()),
        Value::TimestampTz("2024-02-29T23:59+00:00".to_owned()),
        Value::TimestampTz("20240229T235958Z".to_owned()),
        Value::TimestampTz("2024-02-29T23:59:58+24:00".to_owned()),
    ] {
        let update = "UPDATE items SET c_text = $2 WHERE id = $1";
        let params = vec![Some(Value::Int64(1)), Some(value.clone())];
        let bound = database.execute(update.to_owned(), params).await;
        assert!(
            matches!(&bound, Err(Error::Invalid(message)) if message.contains("`$2`")),
            "{value:?} bound: {bound:?}"
        );
    }

    // the edges of each format still bind
    for value in [
        Value::Time("23:59:60".to_owned()),
        Value::TimestampTz("2024-02-29t23:59:58.123456z".to_owned()),
        Value::TimestampTz("2024-02-29 23:59:58-23:59".to_owned()),
    ] {
        let update = "UPDATE items SET c_stamptz = $2 WHERE id = $1";
        let params = vec![Some(Value::Int64(1)), Some(value.clone())];
        let bound = database.execute(update.to_owned(), params).await;
        assert!(matches!(bound, Ok(1)), "{value:?} did not bind: {bound:?}");
    }

    // `TEXT` that is not valid UTF-8 does not read
    let unreadable = database.query("SELECT CAST(X'FF' AS TEXT) AS t".to_owned(), vec![]).await;
    assert!(matches!(unreadable, Err(Error::Other(_))), "{unreadable:?}");
}

async fn select(
    database: &Database, sql: &str, params: Vec<Option<Value>>,
) -> (Vec<String>, Vec<Row>) {
    let selected = database.query(sql.to_owned(), params).await;
    let ResultSet {
        columns,
        rows,
        finished,
    } = selected.unwrap_or_else(|err| panic!("{sql}: {err}"));
    let rows = rows.collect().await;
    finished.await.unwrap_or_else(|err| panic!("{sql} did not finish: {err}"));
    (columns, rows)
}

// `Value` derives no `PartialEq`, so rows compare by their `Debug` text
#[track_caller]
fn assert_same(read: &[Option<Value>], expected: &[Option<Value>]) {
    assert_eq!(format!("{read:?}"), format!("{expected:?}"));
}
