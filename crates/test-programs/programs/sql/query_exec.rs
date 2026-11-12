//! One connection walked end to end over the raw `wasi:sql` bindings: DDL
//! and parameterised inserts through `exec`, a typed read-back through
//! `query` covering every `data-type` variant the default codec round-trips,
//! a filter on a numbered `$1` placeholder, a delete reporting its affected
//! rows, an update whose `$n` placeholders appear out of order, an invalid
//! statement surfacing as an `error` resource with a trace, and a non-UTF-8
//! text value refused on read.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_sql::readwrite::{exec, query};
use omnia_wasi_sql::types::{Connection, DataType, Row, Statement};

omnia_sdk::api::command::export!(scenario);

const CREATED: &str = "2024-01-02T03:04:05Z";
const DAY: &str = "2024-01-02";
const AT: &str = "03:04:05";

async fn scenario() {
    let conn = Connection::open("default".to_owned()).await.expect("open");

    let create = Statement::prepare(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, flag INTEGER, score REAL, \
         count INTEGER, payload BLOB, created TEXT, day TEXT, at TEXT)"
            .to_owned(),
        vec![],
    )
    .await
    .expect("prepare create");
    assert_eq!(exec(&conn, &create).await.expect("create table"), 0);

    // Two inserts between them cover every variant `to_sqlite` stores as a
    // value.
    let insert = "INSERT INTO items (id, name, flag, score, count, payload, created, day, at) \
                  VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";
    let alpha = Statement::prepare(
        insert.to_owned(),
        vec![
            DataType::Int32(Some(1)),
            DataType::Str(Some("alpha".to_owned())),
            DataType::Boolean(Some(true)),
            DataType::Float(Some(1.5)),
            DataType::Uint32(Some(7)),
            DataType::Binary(Some(vec![1, 2, 3])),
            DataType::Timestamp(Some(CREATED.to_owned())),
            DataType::Date(Some(DAY.to_owned())),
            DataType::Time(Some(AT.to_owned())),
        ],
    )
    .await
    .expect("prepare insert");
    assert_eq!(exec(&conn, &alpha).await.expect("insert alpha"), 1);

    let beta = Statement::prepare(
        insert.to_owned(),
        vec![
            DataType::Int64(Some(2)),
            DataType::Str(Some("beta".to_owned())),
            DataType::Boolean(Some(false)),
            DataType::Double(Some(2.25)),
            DataType::Uint64(Some(u64::MAX)),
            DataType::Binary(Some(vec![0xff, 0x00])),
            DataType::Str(Some(CREATED.to_owned())),
            DataType::Str(Some(DAY.to_owned())),
            DataType::Str(Some(AT.to_owned())),
        ],
    )
    .await
    .expect("prepare insert");
    assert_eq!(exec(&conn, &beta).await.expect("insert beta"), 1);

    // Read back: rows are indexed in order, columns keep their names, and
    // every value comes back as the widened storage type `from_sqlite` emits.
    // A `u64` is stored as its raw bits, so `u64::MAX` reads back as -1.
    let select = Statement::prepare(
        "SELECT id, name, flag, score, count, payload, created, day, at FROM items ORDER BY id"
            .to_owned(),
        vec![],
    )
    .await
    .expect("prepare select");
    let rows = query(&conn, &select).await.expect("select");
    assert_eq!(rows.len(), 2);

    let columns = ["id", "name", "flag", "score", "count", "payload", "created", "day", "at"];
    assert_eq!(rows[0].index, "0");
    assert_eq!(names(&rows[0]), columns);
    assert_int(&rows[0], "id", 1);
    assert_str(&rows[0], "name", "alpha");
    assert_int(&rows[0], "flag", 1);
    assert_double(&rows[0], "score", 1.5);
    assert_int(&rows[0], "count", 7);
    assert_binary(&rows[0], "payload", &[1, 2, 3]);
    assert_str(&rows[0], "created", CREATED);
    assert_str(&rows[0], "day", DAY);
    assert_str(&rows[0], "at", AT);

    assert_eq!(rows[1].index, "1");
    assert_eq!(names(&rows[1]), columns);
    assert_int(&rows[1], "id", 2);
    assert_str(&rows[1], "name", "beta");
    assert_int(&rows[1], "flag", 0);
    assert_double(&rows[1], "score", 2.25);
    assert_int(&rows[1], "count", -1);
    assert_binary(&rows[1], "payload", &[0xff, 0x00]);
    assert_str(&rows[1], "created", CREATED);
    assert_str(&rows[1], "day", DAY);
    assert_str(&rows[1], "at", AT);

    // Parameters bind on the query path too.
    let filtered = Statement::prepare(
        "SELECT name FROM items WHERE score > $1".to_owned(),
        vec![DataType::Double(Some(2.0))],
    )
    .await
    .expect("prepare filtered");
    let rows = query(&conn, &filtered).await.expect("filtered select");
    assert_eq!(rows.len(), 1);
    assert_str(&rows[0], "name", "beta");

    // Exec reports the rows it touched.
    let delete = Statement::prepare(
        "DELETE FROM items WHERE id = ?".to_owned(),
        vec![DataType::Int32(Some(2))],
    )
    .await
    .expect("prepare delete");
    assert_eq!(exec(&conn, &delete).await.expect("delete"), 1);

    // A `$n` placeholder binds the value it numbers, wherever it appears.
    let rename = Statement::prepare(
        "UPDATE items SET name = $2 WHERE id = $1".to_owned(),
        vec![DataType::Int32(Some(1)), DataType::Str(Some("renamed".to_owned()))],
    )
    .await
    .expect("prepare rename");
    assert_eq!(exec(&conn, &rename).await.expect("rename"), 1);

    // Preparing is lazy on the host: a bad statement fails at execution with
    // the backend's context chain in the trace.
    let invalid =
        Statement::prepare("SELEKT nonsense".to_owned(), vec![]).await.expect("prepare invalid");
    let error = exec(&conn, &invalid).await.expect_err("invalid SQL executes");
    let trace = error.trace();
    assert!(trace.contains("failed to prepare statement"), "unexpected trace: {trace}");

    // A text value that is not UTF-8 fails the read rather than crossing lossily.
    let invalid_text = Statement::prepare("SELECT CAST(X'FF' AS TEXT) AS t".to_owned(), vec![])
        .await
        .expect("prepare invalid text");
    let error = query(&conn, &invalid_text).await.expect_err("invalid UTF-8 reads");
    let trace = error.trace();
    assert!(trace.contains("invalid UTF-8 in text value"), "unexpected trace: {trace}");
}

fn names(row: &Row) -> Vec<&str> {
    row.fields.iter().map(|field| field.name.as_str()).collect()
}

fn value<'a>(row: &'a Row, name: &str) -> &'a DataType {
    &row.fields.iter().find(|field| field.name == name).unwrap_or_else(|| panic!("{name}")).value
}

fn assert_int(row: &Row, name: &str, expected: i64) {
    match value(row, name) {
        DataType::Int64(Some(actual)) => assert_eq!(*actual, expected, "{name}"),
        other => panic!("{name}: {other:?}"),
    }
}

fn assert_double(row: &Row, name: &str, expected: f64) {
    match value(row, name) {
        DataType::Double(Some(actual)) => {
            assert!((actual - expected).abs() < f64::EPSILON, "{name}")
        }
        other => panic!("{name}: {other:?}"),
    }
}

fn assert_str(row: &Row, name: &str, expected: &str) {
    match value(row, name) {
        DataType::Str(Some(actual)) => assert_eq!(actual, expected, "{name}"),
        other => panic!("{name}: {other:?}"),
    }
}

fn assert_binary(row: &Row, name: &str, expected: &[u8]) {
    match value(row, name) {
        DataType::Binary(Some(actual)) => assert_eq!(actual, expected, "{name}"),
        other => panic!("{name}: {other:?}"),
    }
}
