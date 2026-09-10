//! One connection walked end to end over the raw `wasi:sql` bindings: DDL
//! and parameterised inserts through `exec`, a typed read-back through
//! `query` covering every `data-type` variant the default codec round-trips,
//! a parameterised filter, a delete reporting its affected rows, and an
//! invalid statement surfacing as an `error` resource with a trace.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_sql::readwrite::{exec, query};
use omnia_wasi_sql::types::{Connection, DataType, Row, Statement};

omnia_guest::command!(scenario);

const CREATED: &str = "2024-01-02T03:04:05Z";

async fn scenario() {
    let conn = Connection::open("db".to_owned()).await.expect("open");

    let create = Statement::prepare(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, flag INTEGER, score REAL, \
         count INTEGER, payload BLOB, created TEXT)"
            .to_owned(),
        vec![],
    )
    .await
    .expect("prepare create");
    assert_eq!(exec(&conn, &create).await.expect("create table"), 0);

    // Two inserts between them cover every variant `to_sqlite` stores as a
    // value (date/time have no SQLite mapping and stay out of scope).
    let insert = "INSERT INTO items (id, name, flag, score, count, payload, created) \
                  VALUES (?, ?, ?, ?, ?, ?, ?)";
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
            DataType::Uint64(Some(9)),
            DataType::Binary(Some(vec![0xff, 0x00])),
            DataType::Str(Some(CREATED.to_owned())),
        ],
    )
    .await
    .expect("prepare insert");
    assert_eq!(exec(&conn, &beta).await.expect("insert beta"), 1);

    // Read back: rows are indexed in order, columns keep their names, and
    // every value comes back as the widened storage type `from_sqlite` emits.
    let select = Statement::prepare(
        "SELECT id, name, flag, score, count, payload, created FROM items ORDER BY id".to_owned(),
        vec![],
    )
    .await
    .expect("prepare select");
    let rows = query(&conn, &select).await.expect("select");
    assert_eq!(rows.len(), 2);

    let columns = ["id", "name", "flag", "score", "count", "payload", "created"];
    assert_eq!(rows[0].index, "0");
    assert_eq!(names(&rows[0]), columns);
    assert_int(&rows[0], "id", 1);
    assert_str(&rows[0], "name", "alpha");
    assert_int(&rows[0], "flag", 1);
    assert_double(&rows[0], "score", 1.5);
    assert_int(&rows[0], "count", 7);
    assert_binary(&rows[0], "payload", &[1, 2, 3]);
    assert_str(&rows[0], "created", CREATED);

    assert_eq!(rows[1].index, "1");
    assert_eq!(names(&rows[1]), columns);
    assert_int(&rows[1], "id", 2);
    assert_str(&rows[1], "name", "beta");
    assert_int(&rows[1], "flag", 0);
    assert_double(&rows[1], "score", 2.25);
    assert_int(&rows[1], "count", 9);
    assert_binary(&rows[1], "payload", &[0xff, 0x00]);
    assert_str(&rows[1], "created", CREATED);

    // Parameters bind on the query path too.
    let filtered = Statement::prepare(
        "SELECT name FROM items WHERE score > ?".to_owned(),
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

    // Preparing is lazy on the host: a bad statement fails at execution with
    // the backend's context chain in the trace.
    let invalid =
        Statement::prepare("SELEKT nonsense".to_owned(), vec![]).await.expect("prepare invalid");
    let error = exec(&conn, &invalid).await.expect_err("invalid SQL executes");
    let trace = error.trace();
    assert!(trace.contains("failed to prepare statement"), "unexpected trace: {trace}");
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
