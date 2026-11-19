//! The `Connection` defaults, over the suite's scripted backend, which
//! implements only `query`, `execute`, and `batch`, as Postgres does.
//!
//! - An `INSERT` binds one value of every case, and the suite asserts the
//!   seam value each became.
//! - Rows shaped like Postgres's arrive in the case each seam value maps to.
//! - The default `select` names no columns when no row is selected.
//! - The default `begin` and `commit` reach the backend as `BEGIN` and
//!   `COMMIT` through `execute`, on the connection `store::open` returned.
//! - A `ROLLBACK` follows a failed statement, and another follows a dropped
//!   transaction.
//! - Every spelling of transaction control the host refuses is `invalid`, and
//!   reaches no backend.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::sql::database::{Database, Statement};
use omnia_sdk::sql::types::{Error, ResultSet, Row, Value};
use omnia_sdk::sql::{store, transactions};

omnia_sdk::api::command::export!(scenario);

const DATE: &str = "2024-02-29";
const TIME: &str = "23:59:58.5";
const STAMP: &str = "2024-02-29 23:59:58";
const STAMPTZ: &str = "2024-02-29T23:59:58+00:00";
const INSERT: &str = "INSERT INTO single (id) VALUES ($1)";

// each refused before it reaches a backend
const REFUSED: [&str; 11] = [
    "BEGIN",
    " ;; begin transaction",
    "-- a line comment\nCOMMIT",
    "/* outer /* nested */ still a comment */ ROLLBACK TO SAVEPOINT s",
    "Start Transaction",
    "END",
    "abort",
    "SAVEPOINT s",
    "RELEASE SAVEPOINT s",
    "PREPARE TRANSACTION 'held'",
    "prepare\ttransaction 'held'",
];

async fn scenario() {
    let database = store::open("default".to_owned()).await.expect("open");

    // one value of every case binds
    let insert = "INSERT INTO typed VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";
    let every_case = vec![
        Some(Value::Boolean(true)),
        Some(Value::Int32(-7)),
        Some(Value::Int64(i64::MIN)),
        Some(Value::Float32(1.5)),
        Some(Value::Float64(-2.25)),
        Some(Value::Text("héllo".to_owned())),
        Some(Value::Binary(vec![0, 255])),
        Some(Value::Date(DATE.to_owned())),
        Some(Value::Time(TIME.to_owned())),
        Some(Value::Timestamp(STAMP.to_owned())),
        Some(Value::TimestampTz(STAMPTZ.to_owned())),
        None,
    ];
    assert_eq!(database.execute(insert.to_owned(), every_case).await.expect("insert"), 1);

    // rows shaped like Postgres's arrive in the case each value maps to
    let (columns, rows) = select(&database, "SELECT * FROM typed").await;
    let names = [
        "c_int32",
        "c_uint32",
        "c_uint64",
        "c_float",
        "c_date",
        "c_time",
        "c_stamp",
        "c_stamptz",
        "c_null_int",
        "c_null_date",
    ];
    assert_eq!(columns, names);
    let expected = [vec![
        Some(Value::Int32(-7)),
        Some(Value::Int64(u32::MAX.into())),
        Some(Value::Text(u64::MAX.to_string())),
        Some(Value::Float32(1.5)),
        Some(Value::Date(DATE.to_owned())),
        Some(Value::Time(TIME.to_owned())),
        Some(Value::Timestamp(STAMP.to_owned())),
        Some(Value::TimestampTz(STAMPTZ.to_owned())),
        None,
        None,
    ]];
    assert_eq!(format!("{rows:?}"), format!("{expected:?}"));

    // the default `select` names no columns without a row
    let (columns, rows) = select(&database, "SELECT * FROM typed WHERE false").await;
    assert!(columns.is_empty() && rows.is_empty(), "selected {columns:?} and {rows:?}");

    // a transaction commits on the connection `open` returned
    let opened = store::open("default".to_owned()).await.expect("open");
    let committed = transactions::begin(&opened).await.expect("begin");
    let isolation = "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE";
    assert_eq!(committed.execute(isolation.to_owned(), vec![]).await.expect("isolation"), 0);
    assert_eq!(committed.execute(INSERT.to_owned(), id(1)).await.expect("insert"), 1);
    committed.commit().await.expect("commit");

    // a failed statement is followed by a rollback
    let failing = transactions::begin(&database).await.expect("begin");
    let failed = failing.execute("INSERT INTO missing (id) VALUES ($1)".to_owned(), id(1)).await;
    assert!(matches!(failed, Err(Error::Other(_))), "{failed:?}");
    let commit = failing.commit().await;
    assert!(matches!(commit, Err(Error::Invalid(_))), "{commit:?}");

    // so is a dropped transaction
    let dropped = transactions::begin(&database).await.expect("begin");
    assert_eq!(dropped.execute(INSERT.to_owned(), id(2)).await.expect("insert"), 1);
    drop(dropped);

    // transaction control is refused on every path
    for sql in REFUSED {
        let refused = database.execute(sql.to_owned(), vec![]).await;
        assert!(matches!(refused, Err(Error::Invalid(_))), "`{sql}` ran: {refused:?}");
    }
    let refused = database.query("COMMIT".to_owned(), vec![]).await;
    assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
    let batch = vec![
        Statement {
            sql: INSERT.to_owned(),
            params: id(3),
        },
        Statement {
            sql: "COMMIT".to_owned(),
            params: vec![],
        },
    ];
    let refused = database.batch(batch).await;
    assert!(
        matches!(&refused, Err(Error::Invalid(message)) if message.starts_with("statement 2 ")),
        "{refused:?}"
    );
    assert!(database.batch(vec![]).await.expect("an empty batch").is_empty());
}

fn id(id: i32) -> Vec<Option<Value>> {
    vec![Some(Value::Int32(id))]
}

async fn select(database: &Database, sql: &str) -> (Vec<String>, Vec<Row>) {
    let selected = database.query(sql.to_owned(), vec![]).await;
    let ResultSet {
        columns,
        rows,
        finished,
    } = selected.unwrap_or_else(|err| panic!("{sql}: {err}"));
    let rows = rows.collect().await;
    finished.await.unwrap_or_else(|err| panic!("{sql} did not finish: {err}"));
    (columns, rows)
}
