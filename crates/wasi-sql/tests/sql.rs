//! End-to-end tests for `omnia:sql`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. The guest
//! asserts what it observes across the boundary, and traps on failure.
//!
//! - Most scenarios run against the in-memory `SQLite` default, and the suite
//!   asserts what persisted in the shared database after the run.
//! - `sql_backend_defaults` runs against a scripted backend that implements
//!   only the `Connection` methods without a default, and the suite asserts
//!   every call that reached it.
//! - `sql_error_codes` runs against a backend that fails by database name.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omnia::{Connect as _, ExitStatus};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_sql::{
    ConnectOptions, Connection, Error, Field, FutureResult, Row, SqlClient, SqlDefault, Statement,
    Value, WasiSql,
};
use tokio::sync::Notify;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_sql!();

async fn run_guest<S: SqlClient + Clone>(wasm: &str, sql: S) {
    let backends = Backends::defaults().await.sql(sql);
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiSql, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

// `default` and each of `databases`, every one a private in-memory database
async fn in_memory(databases: &[&str]) -> SqlDefault {
    let options = ConnectOptions {
        database: ":memory:".to_owned(),
        databases: databases
            .iter()
            .map(|name| ((*name).to_owned(), ":memory:".to_owned()))
            .collect(),
    };
    SqlDefault::connect_with(options).await.expect("in-memory sqlite")
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
            let Value::Str(Some(name)) = &field.value else {
                panic!("unexpected value: {:?}", field.value);
            };
            assert_eq!(field.name, "name");
            name.clone()
        })
        .collect()
}

#[tokio::test]
async fn sql_database() {
    let sql = in_memory(&[]).await;

    run_guest(test_programs::SQL_DATABASE, sql.clone()).await;

    // Persisted state: the updates the host refused left the text as inserted.
    let db = sql.open("default".to_owned()).await.expect("open");
    let rows = db.query("SELECT c_text FROM items ORDER BY id".to_owned(), vec![]).await;
    let texts: Vec<Value> =
        rows.expect("query").into_iter().flat_map(|row| row.fields).map(|f| f.value).collect();
    assert_eq!(texts, [Value::Str(Some("héllo".to_owned())), Value::Str(None)]);
}

#[tokio::test]
async fn sql_transactions() {
    let sql = in_memory(&[]).await;

    run_guest(test_programs::SQL_TRANSACTIONS, sql.clone()).await;

    // Persisted state: only what committed, and what ran after the refusal.
    assert_eq!(names(&sql, "default").await, ["committed", "outlived", "usable"]);
}

#[tokio::test]
async fn sql_sql_db() {
    // `omnia_test::contract::sql` runs over one private `default` database
    run_guest(test_programs::SQL_SQL_DB, in_memory(&[]).await).await;
}

#[tokio::test]
async fn sql_bindings() {
    let sql = in_memory(&["second"]).await;

    run_guest(test_programs::SQL_BINDINGS, sql.clone()).await;

    // Persisted state: the declared name is its own database, which holds
    // only what the guest wrote through it.
    assert_eq!(names(&sql, "second").await, ["delta", "gamma"]);
}

const DATE: &str = "2024-02-29";
const TIME: &str = "23:59:58.5";
const STAMP: &str = "2024-02-29 23:59:58";
const STAMPTZ: &str = "2024-02-29T23:59:58+00:00";
const INSERT_TYPED: &str =
    "INSERT INTO typed VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";
const INSERT: &str = "INSERT INTO single (id) VALUES ($1)";
const INSERT_MISSING: &str = "INSERT INTO missing (id) VALUES ($1)";
const ISOLATION: &str = "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE";
const SELECT_TYPED: &str = "SELECT * FROM typed";
const SELECT_NONE: &str = "SELECT * FROM typed WHERE false";

#[tokio::test]
async fn sql_backend_defaults() {
    let scripted = Scripted::default();

    run_guest(test_programs::SQL_BACKEND_DEFAULTS, scripted.clone()).await;

    let bound = [
        Value::Boolean(Some(true)),
        Value::Int32(Some(-7)),
        Value::Int64(Some(i64::MIN)),
        Value::Float(Some(1.5)),
        Value::Double(Some(-2.25)),
        Value::Str(Some("héllo".to_owned())),
        Value::Binary(Some(vec![0, 255])),
        Value::Date(Some(DATE.to_owned())),
        Value::Time(Some(TIME.to_owned())),
        Value::Timestamp(Some(STAMP.to_owned())),
        Value::Timestamp(Some(STAMPTZ.to_owned())),
        Value::Str(None),
    ];
    let expected = [
        Reached::new(1, "execute", INSERT_TYPED, &bound),
        Reached::new(2, "query", SELECT_TYPED, &[]),
        Reached::new(3, "query", SELECT_NONE, &[]),
        Reached::new(4, "execute", "BEGIN", &[]),
        Reached::new(4, "execute", ISOLATION, &[]),
        Reached::new(4, "execute", INSERT, &[Value::Int32(Some(1))]),
        Reached::new(4, "execute", "COMMIT", &[]),
        Reached::new(5, "execute", "BEGIN", &[]),
        Reached::new(5, "execute", INSERT_MISSING, &[Value::Int32(Some(1))]),
        Reached::new(5, "execute", "ROLLBACK", &[]),
        Reached::new(6, "execute", "BEGIN", &[]),
        Reached::new(6, "execute", INSERT, &[Value::Int32(Some(2))]),
        Reached::new(6, "execute", "ROLLBACK", &[]),
    ];
    assert_eq!(scripted.settled(expected.len()).await, expected);
}

// implements only the `Connection` methods without a default, as Postgres
// does, and records every call that reaches a connection, numbered in the
// order `open` returned it
#[derive(Clone, Debug, Default)]
struct Scripted {
    opened: Arc<AtomicUsize>,
    reached: Arc<Mutex<Vec<Reached>>>,
    recorded: Arc<Notify>,
}

impl Scripted {
    // the dropped transaction's rollback may reach the backend after the
    // guest returns
    async fn settled(&self, count: usize) -> Vec<Reached> {
        let settled = async {
            loop {
                let reached = self.reached.lock().expect("reached calls").clone();
                if reached.len() >= count {
                    return reached;
                }
                self.recorded.notified().await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), settled).await.expect("the backend settles")
    }
}

impl SqlClient for Scripted {
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>> {
        let connection = ScriptedConnection {
            number: self.opened.fetch_add(1, Ordering::Relaxed) + 1,
            backend: self.clone(),
        };
        Box::pin(async move {
            anyhow::ensure!(name == "default", "the scenario opens only `default`, not `{name}`");
            Ok(Arc::new(connection) as Arc<dyn Connection>)
        })
    }
}

#[derive(Debug)]
struct ScriptedConnection {
    number: usize,
    backend: Scripted,
}

impl ScriptedConnection {
    fn reach(&self, call: &'static str, sql: String, params: Vec<Value>) {
        let reached = Reached {
            connection: self.number,
            call,
            sql,
            params,
        };
        self.backend.reached.lock().expect("reached calls").push(reached);
        self.backend.recorded.notify_one();
    }
}

impl Connection for ScriptedConnection {
    fn query(&self, sql: String, params: Vec<Value>) -> FutureResult<Vec<Row>> {
        let rows = match sql.as_str() {
            SELECT_TYPED => Ok(vec![typed_row()]),
            SELECT_NONE => Ok(vec![]),
            _ => Err(anyhow::anyhow!("unscripted query `{sql}`")),
        };
        self.reach("query", sql, params);
        Box::pin(async move { rows })
    }

    fn execute(&self, sql: String, params: Vec<Value>) -> FutureResult<u64> {
        let changed = match sql.as_str() {
            "BEGIN" | "COMMIT" | "ROLLBACK" | ISOLATION => Ok(0),
            INSERT_TYPED | INSERT => Ok(1),
            INSERT_MISSING => Err(anyhow::anyhow!("relation \"missing\" does not exist")),
            _ => Err(anyhow::anyhow!("unscripted statement `{sql}`")),
        };
        self.reach("execute", sql, params);
        Box::pin(async move { changed })
    }

    fn batch(&self, statements: Vec<Statement>) -> FutureResult<Vec<u64>> {
        for statement in statements {
            self.reach("batch", statement.sql, statement.params);
        }
        Box::pin(async { Err(anyhow::anyhow!("unscripted batch")) })
    }
}

// a row as Postgres returns one, with a typed `NULL` in each null column
fn typed_row() -> Row {
    let values = [
        ("c_int32", Value::Int32(Some(-7))),
        ("c_uint32", Value::Uint32(Some(u32::MAX))),
        ("c_uint64", Value::Uint64(Some(u64::MAX))),
        ("c_float", Value::Float(Some(1.5))),
        ("c_date", Value::Date(Some(DATE.to_owned()))),
        ("c_time", Value::Time(Some(TIME.to_owned()))),
        ("c_stamp", Value::Timestamp(Some(STAMP.to_owned()))),
        ("c_stamptz", Value::Timestamp(Some(STAMPTZ.to_owned()))),
        ("c_null_int", Value::Int32(None)),
        ("c_null_date", Value::Date(None)),
    ];
    Row {
        fields: values
            .into_iter()
            .map(|(name, value)| Field {
                name: name.to_owned(),
                value,
            })
            .collect(),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Reached {
    connection: usize,
    call: &'static str,
    sql: String,
    params: Vec<Value>,
}

impl Reached {
    fn new(connection: usize, call: &'static str, sql: &str, params: &[Value]) -> Self {
        Self {
            connection,
            call,
            sql: sql.to_owned(),
            params: params.to_vec(),
        }
    }
}

// fails `open` with the kind a database is named for, serves `untransacted`
// with no transactions, delegates any other name, and records each name so
// the run's exchanges are pinned
#[derive(Clone, Debug)]
struct Faulting {
    inner: SqlDefault,
    opened: Arc<Mutex<Vec<String>>>,
}

impl SqlClient for Faulting {
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>> {
        self.opened.lock().expect("opened databases").push(name.clone());
        let fault = match name.as_str() {
            "denied" => Error::Denied("denied by the suite".to_owned()).into(),
            "unsupported" => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "unavailable" => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            "other" => anyhow::anyhow!("an untyped backend failure"),
            "untransacted" => {
                return Box::pin(async { Ok(Arc::new(Untransacted) as Arc<dyn Connection>) });
            }
            _ => return self.inner.open(name),
        };
        Box::pin(async move { Err(fault) })
    }
}

// the scenario runs no statement on it
#[derive(Debug)]
struct Untransacted;

impl Connection for Untransacted {
    fn query(&self, sql: String, _: Vec<Value>) -> FutureResult<Vec<Row>> {
        Box::pin(async move { Err(anyhow::anyhow!("unscripted query `{sql}`")) })
    }

    fn execute(&self, sql: String, _: Vec<Value>) -> FutureResult<u64> {
        Box::pin(async move { Err(anyhow::anyhow!("unscripted statement `{sql}`")) })
    }

    fn batch(&self, _: Vec<Statement>) -> FutureResult<Vec<u64>> {
        Box::pin(async { Err(anyhow::anyhow!("unscripted batch")) })
    }

    fn begin(&self) -> FutureResult<()> {
        Box::pin(async {
            Err(Error::Unsupported("no transactions in the suite".to_owned()).into())
        })
    }
}

#[tokio::test]
async fn sql_error_codes() {
    let faulting = Faulting {
        inner: in_memory(&[]).await,
        opened: Arc::default(),
    };

    run_guest(test_programs::SQL_ERROR_CODES, faulting.clone()).await;

    let opened = faulting.opened.lock().expect("opened databases").clone();
    assert_eq!(
        opened,
        [
            "undeclared",
            "default",
            "default",
            "default",
            "default",
            "denied",
            "unsupported",
            "unavailable",
            "other",
            "untransacted",
        ]
    );
}
