//! Default `SQLite` implementation of `omnia:sql`.
//!
//! This is a lightweight implementation for development use only.

use std::collections::{BTreeMap, HashMap};
use std::env::{self, VarError};
use std::iter;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use futures::FutureExt;
use omnia_core::Connect;
use parking_lot::{Condvar, Mutex};
use rusqlite::types::{Value as SqliteValue, ValueRef};
use rusqlite::{Connection as SqliteConnection, ErrorCode, Statement as SqliteStatement};
use tracing::instrument;

use crate::host::resource::{Connection, FutureResult};
use crate::host::{Error, Field, ResultSet, Row, SqlClient, Statement, Value};

const DEFAULT: &str = "default";

/// The databases a [`SqlDefault`] opens, by the name a guest opens each under.
///
/// [`FromEnv`](omnia_core::FromEnv) reads them from the environment:
///
/// - `SQL_DATABASE` is the URI of the `default` database, one in-memory
///   database shared across the process when unset.
/// - `SQL_DATABASES` lists the further names, comma-separated.
/// - `SQL_DATABASE__<NAME>` is the URI of the listed name `<name>`, a private
///   in-memory database when unset.
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    /// The `SQLite` URI of the `default` database.
    pub database: String,

    /// The `SQLite` URI of each further database, by name.
    pub databases: BTreeMap<String, String>,
}

impl omnia_core::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        let database = var("SQL_DATABASE")?.unwrap_or_else(|| "file::memory:?cache=shared".into());
        let mut databases = BTreeMap::new();
        for name in var("SQL_DATABASES")?.unwrap_or_default().split(',').map(str::trim) {
            if name.is_empty() {
                continue;
            }
            let uri = var(&format!("SQL_DATABASE__{}", name.to_ascii_uppercase()))?;
            databases.insert(name.to_owned(), uri.unwrap_or_else(|| ":memory:".into()));
        }
        Ok(Self { database, databases })
    }
}

fn var(key: &str) -> Result<Option<String>> {
    match env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(err) => Err(err).with_context(|| format!("reading `{key}`")),
    }
}

/// Default `SQLite` backend for `omnia:sql`.
///
/// It holds one connection per declared database, and opening any other name
/// fails with [`Error::NotFound`]. Names match case-insensitively. Dates,
/// times, and timestamps are stored as the text they are given.
///
/// Every handle on a database shares its connection. While a transaction is
/// open, a call on any other handle waits for it to end, and fails with
/// [`Error::Unavailable`] after five seconds.
#[derive(Debug, Clone)]
pub struct SqlDefault {
    // keyed by upper-cased name
    databases: Arc<HashMap<String, Arc<Database>>>,
}

impl Connect for SqlDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        let declared = iter::once((DEFAULT, &options.database))
            .chain(options.databases.iter().map(|(name, uri)| (name.as_str(), uri)));

        // open now, so a bad database path fails connect rather than the first query
        let mut databases = HashMap::new();
        for (name, uri) in declared {
            tracing::debug!("opening SQLite database `{name}` at: {uri}");
            let conn = SqliteConnection::open(uri)
                .with_context(|| format!("failed to open SQLite database `{name}`"))?;
            let database = Arc::new(Database::new(name.to_owned(), conn));
            let replaced = databases.insert(name.to_ascii_uppercase(), database);
            ensure!(replaced.is_none(), "SQLite database `{name}` is declared twice");
        }

        Ok(Self {
            databases: Arc::new(databases),
        })
    }
}

impl SqlClient for SqlDefault {
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>> {
        tracing::trace!("opening SQL connection `{name}`");
        let database = self.databases.get(&name.to_ascii_uppercase()).map(Arc::clone);

        async move {
            let Some(database) = database else {
                return Err(Error::NotFound(format!("database `{name}` is not declared")).into());
            };
            Ok(Arc::new(SqliteConn::new(database)) as Arc<dyn Connection>)
        }
        .boxed()
    }
}

// How long a call waits for another handle's transaction to end, which
// matches rusqlite's busy timeout.
const TRANSACTION_WAIT: Duration = Duration::from_secs(5);

// One declared database, whose connection every handle shares.
#[derive(Debug)]
struct Database {
    name: String,
    session: Mutex<Session>,
    released: Condvar,
    // a copy of `Session::holder`, 0 for none, so a handle that holds no
    // transaction drops without taking the lock
    holder: AtomicU64,
    next_handle: AtomicU64,
}

#[derive(Debug)]
struct Session {
    conn: SqliteConnection,
    holder: Option<u64>,
}

impl Database {
    const fn new(name: String, conn: SqliteConnection) -> Self {
        Self {
            name,
            session: Mutex::new(Session { conn, holder: None }),
            released: Condvar::new(),
            holder: AtomicU64::new(0),
            next_handle: AtomicU64::new(1),
        }
    }

    // Runs `work` once no other handle's transaction holds the database.
    fn with<T>(&self, handle: u64, work: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
        let mut session = self.session.lock();
        let held = |session: &mut Session| session.holder.is_some_and(|holder| holder != handle);
        let deadline = Instant::now() + TRANSACTION_WAIT;
        if self.released.wait_while_until(&mut session, held, deadline).timed_out() {
            let name = &self.name;
            return Err(
                Error::Unavailable(format!("database `{name}` is held by a transaction")).into()
            );
        }
        let outcome = work(&mut session);
        drop(session);
        outcome
    }

    fn hold(&self, session: &mut Session, handle: u64) {
        session.holder = Some(handle);
        self.holder.store(handle, Ordering::Release);
    }

    fn release(&self, session: &mut Session) {
        session.holder = None;
        self.holder.store(0, Ordering::Release);
        self.released.notify_all();
    }
}

// The calls in flight share the handle, so the rollback a drop runs waits for
// the last of them to finish.
#[derive(Debug)]
struct SqliteConn(Arc<Handle>);

#[derive(Debug)]
struct Handle {
    database: Arc<Database>,
    id: u64,
}

impl SqliteConn {
    fn new(database: Arc<Database>) -> Self {
        let id = database.next_handle.fetch_add(1, Ordering::Relaxed);
        Self(Arc::new(Handle { database, id }))
    }

    // rusqlite blocks, and waits on other handles' transactions, so its work
    // never pins an executor thread
    fn blocking<T: Send + 'static>(
        &self, work: impl FnOnce(&Handle) -> Result<T> + Send + 'static,
    ) -> FutureResult<T> {
        let handle = Arc::clone(&self.0);
        async move {
            tokio::task::spawn_blocking(move || work(&handle))
                .await
                .context("SQLite task panicked")?
        }
        .boxed()
    }
}

impl Connection for SqliteConn {
    fn query(&self, sql: String, params: Vec<Value>) -> FutureResult<Vec<Row>> {
        let selected = self.select(sql, params);
        async move { Ok(selected.await?.rows) }.boxed()
    }

    fn execute(&self, sql: String, params: Vec<Value>) -> FutureResult<u64> {
        tracing::trace!("executing statement: {sql}");
        self.blocking(move |handle| handle.run(|conn| execute(conn, &sql, &params)))
    }

    fn batch(&self, statements: Vec<Statement>) -> FutureResult<Vec<u64>> {
        tracing::trace!("executing a batch of {} statements", statements.len());
        self.blocking(move |handle| {
            handle.run(|conn| {
                let tx = conn.transaction().map_err(|err| failure(err, "failed to begin batch"))?;
                let counts = statements
                    .iter()
                    .map(|statement| execute(&tx, &statement.sql, &statement.params))
                    .collect::<Result<Vec<_>>>()?;
                tx.commit().map_err(|err| failure(err, "failed to commit batch"))?;
                Ok(counts)
            })
        })
    }

    fn select(&self, sql: String, params: Vec<Value>) -> FutureResult<ResultSet> {
        tracing::trace!("executing query: {sql}");
        self.blocking(move |handle| handle.run(|conn| select(conn, &sql, &params)))
    }

    fn begin(&self) -> FutureResult<()> {
        self.blocking(Handle::begin)
    }

    fn commit(&self) -> FutureResult<()> {
        self.blocking(Handle::commit)
    }

    fn rollback(&self) -> FutureResult<()> {
        self.blocking(Handle::rollback)
    }
}

impl Handle {
    // SQLite can run transaction control the host's check misses, since it
    // skips a leading byte-order mark and does not nest block comments, so a
    // call is refused for what it did to a transaction it does not own.
    fn run<T>(&self, work: impl FnOnce(&mut SqliteConnection) -> Result<T>) -> Result<T> {
        self.database.with(self.id, |session| {
            let held = session.holder == Some(self.id);
            let outcome = work(&mut session.conn);

            match (held, session.conn.is_autocommit()) {
                (false, false) => {
                    drop(session.conn.execute_batch("ROLLBACK"));
                    Err(control("opened"))
                }
                // SQLite also ends a transaction itself, on some failures
                (true, true) => {
                    self.database.release(session);
                    outcome?;
                    Err(control("ended"))
                }
                _ => outcome,
            }
        })
    }

    fn begin(&self) -> Result<()> {
        self.database.with(self.id, |session| {
            session.conn.execute_batch("BEGIN").map_err(|err| failure(err, "failed to begin"))?;
            self.database.hold(session, self.id);
            Ok(())
        })
    }

    fn commit(&self) -> Result<()> {
        self.database.with(self.id, |session| {
            if session.holder != Some(self.id) {
                return Err(Error::Invalid("no transaction is open to commit".to_owned()).into());
            }
            self.finish(session, "COMMIT")
        })
    }

    fn rollback(&self) -> Result<()> {
        self.database.with(self.id, |session| {
            if session.holder != Some(self.id) {
                return Ok(());
            }
            self.finish(session, "ROLLBACK")
        })
    }

    // A failed `COMMIT` leaves the transaction open, for the rollback that
    // follows it.
    fn finish(&self, session: &mut Session, sql: &'static str) -> Result<()> {
        let finished = session.conn.execute_batch(sql);
        if session.conn.is_autocommit() {
            self.database.release(session);
        }
        finished.map_err(|err| failure(err, &format!("failed to run `{sql}`")))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if self.database.holder.load(Ordering::Acquire) == self.id {
            drop(self.rollback());
        }
    }
}

fn control(did: &str) -> anyhow::Error {
    Error::Invalid(format!("the statement {did} a transaction, which only `transactions` may do"))
        .into()
}

// `SQLITE_BUSY` and `SQLITE_LOCKED` mean another connection holds the
// database, so a later call may succeed.
fn failure(err: rusqlite::Error, context: &str) -> anyhow::Error {
    match err.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            Error::Unavailable(format!("{context}: {err}")).into()
        }
        _ => anyhow::Error::new(err).context(context.to_owned()),
    }
}

fn select(conn: &SqliteConnection, sql: &str, params: &[Value]) -> Result<ResultSet> {
    let mut stmt = prepare(conn, sql, params)?;
    let columns: Vec<String> = stmt.column_names().map(ToString::to_string).collect();

    let mut rows = stmt.raw_query();
    let mut selected = Vec::new();
    while let Some(row) = rows.next().map_err(|err| failure(err, "failed to fetch row"))? {
        let mut fields = Vec::with_capacity(columns.len());
        for (i, name) in columns.iter().enumerate() {
            let value = row.get_ref(i).context("failed to get column value")?;
            fields.push(Field {
                name: name.clone(),
                value: from_sqlite(value)?,
            });
        }
        selected.push(Row { fields });
    }

    Ok(ResultSet {
        columns,
        rows: selected,
    })
}

// `changes` still counts the last insert, update, or delete after DDL, so it
// is read only when `total_changes` moved.
fn execute(conn: &SqliteConnection, sql: &str, params: &[Value]) -> Result<u64> {
    let mut stmt = prepare(conn, sql, params)?;
    let before = conn.total_changes();
    stmt.raw_execute().map_err(|err| failure(err, "failed to execute statement"))?;
    Ok(if conn.total_changes() == before { 0 } else { conn.changes() })
}

// a statement that does not prepare or bind is the caller's to fix
fn prepare<'conn>(
    conn: &'conn SqliteConnection, sql: &str, params: &[Value],
) -> Result<SqliteStatement<'conn>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|err| Error::Invalid(format!("failed to prepare statement: {err}")))?;
    bind(&mut stmt, params)
        .map_err(|err| Error::Invalid(format!("failed to bind parameters: {err:#}")))?;
    Ok(stmt)
}

// `SQLite` numbers `$n` slots by first appearance, not by `n`: positional
// binding would hand `$2` the first value in `SET name = $2 WHERE id = $1`.
fn bind(stmt: &mut SqliteStatement<'_>, params: &[Value]) -> Result<()> {
    let count = stmt.parameter_count();
    ensure!(count == params.len(), "the statement takes {count} parameters, not {}", params.len());

    for slot in 1..=count {
        let value = match stmt.parameter_name(slot) {
            None => &params[slot - 1],
            Some(name) => {
                let number = name
                    .strip_prefix(['$', '?'])
                    .and_then(|number| number.parse::<usize>().ok())
                    .with_context(|| {
                        format!("unsupported placeholder `{name}`; use `?`, `?n`, or `$n`")
                    })?;
                number
                    .checked_sub(1)
                    .and_then(|index| params.get(index))
                    .with_context(|| format!("placeholder `{name}` has no supplied value"))?
            }
        };
        stmt.raw_bind_parameter(slot, to_sqlite(value))?;
    }

    Ok(())
}

// A `u64` is stored as its raw bits, the `SQLite` convention: its integers
// are always signed 64-bit, and readers reinterpret.
fn to_sqlite(value: &Value) -> SqliteValue {
    match value {
        Value::Int32(Some(i)) => SqliteValue::Integer(i64::from(*i)),
        Value::Int64(Some(i)) => SqliteValue::Integer(*i),
        Value::Uint32(Some(u)) => SqliteValue::Integer(i64::from(*u)),
        Value::Uint64(Some(u)) => SqliteValue::Integer(u.cast_signed()),
        Value::Float(Some(f)) => SqliteValue::Real(f64::from(*f)),
        Value::Double(Some(f)) => SqliteValue::Real(*f),
        Value::Boolean(Some(b)) => SqliteValue::Integer(i64::from(*b)),
        Value::Str(Some(s))
        | Value::Date(Some(s))
        | Value::Time(Some(s))
        | Value::Timestamp(Some(s)) => SqliteValue::Text(s.clone()),
        Value::Binary(Some(b)) => SqliteValue::Blob(b.clone()),
        Value::Int32(None)
        | Value::Int64(None)
        | Value::Uint32(None)
        | Value::Uint64(None)
        | Value::Float(None)
        | Value::Double(None)
        | Value::Boolean(None)
        | Value::Str(None)
        | Value::Date(None)
        | Value::Time(None)
        | Value::Timestamp(None)
        | Value::Binary(None) => SqliteValue::Null,
    }
}

fn from_sqlite(value: ValueRef) -> Result<Value> {
    match value {
        ValueRef::Null => Ok(Value::Str(None)),
        ValueRef::Integer(i) => Ok(Value::Int64(Some(i))),
        ValueRef::Real(f) => Ok(Value::Double(Some(f))),
        ValueRef::Text(t) => {
            let s = std::str::from_utf8(t).context("invalid UTF-8 in text value")?;
            Ok(Value::Str(Some(s.to_string())))
        }
        ValueRef::Blob(b) => Ok(Value::Binary(Some(b.to_vec()))),
    }
}
