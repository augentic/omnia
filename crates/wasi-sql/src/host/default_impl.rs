//! Default `SQLite` implementation of `omnia:sql` and the `wasi:sql` draft.
//!
//! This is a lightweight implementation for development use only.

use std::collections::{BTreeMap, HashMap};
use std::env::{self, VarError};
use std::iter;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use futures::FutureExt;
use omnia_core::Connect;
use rusqlite::types::{Value as SqliteValue, ValueRef};
use rusqlite::{Connection as SqliteConnection, Statement as SqliteStatement};
use tracing::instrument;

use crate::host::resource::{Connection, FutureResult};
use crate::host::{Error, Field, Row, SqlClient, Statement, Value};

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

/// Default `SQLite` backend for `omnia:sql` and the `wasi:sql` draft.
///
/// It holds one connection per declared database, and opening any other name
/// fails with [`Error::NotFound`]. Names match case-insensitively. Dates,
/// times, and timestamps are stored as the text they are given.
#[derive(Debug, Clone)]
pub struct SqlDefault {
    // keyed by upper-cased name; the mutex is there because `rusqlite::Connection` is not `Sync`
    databases: Arc<HashMap<String, Arc<parking_lot::Mutex<SqliteConnection>>>>,
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
            let conn = Arc::new(parking_lot::Mutex::new(conn));
            let replaced = databases.insert(name.to_ascii_uppercase(), conn);
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
        let conn = self.databases.get(&name.to_ascii_uppercase()).map(Arc::clone);

        async move {
            let Some(conn) = conn else {
                return Err(Error::NotFound(format!("database `{name}` is not declared")).into());
            };
            Ok(Arc::new(SqliteConn { conn }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct SqliteConn {
    conn: Arc<parking_lot::Mutex<SqliteConnection>>,
}

impl SqliteConn {
    // rusqlite blocks, and holds the mutex, so it never pins an executor thread
    fn blocking<T: Send + 'static>(
        &self, work: impl FnOnce(&mut SqliteConnection) -> Result<T> + Send + 'static,
    ) -> FutureResult<T> {
        let conn = Arc::clone(&self.conn);
        async move {
            tokio::task::spawn_blocking(move || work(&mut conn.lock()))
                .await
                .context("SQLite task panicked")?
        }
        .boxed()
    }
}

impl Connection for SqliteConn {
    fn query(&self, sql: String, params: Vec<Value>) -> FutureResult<Vec<Row>> {
        tracing::trace!("executing query: {sql}");
        self.blocking(move |conn| {
            let mut stmt = prepare(conn, &sql, &params)?;
            let names: Vec<String> = stmt.column_names().map(ToString::to_string).collect();

            let mut rows = stmt.raw_query();
            let mut selected = Vec::new();
            while let Some(row) = rows.next().context("failed to fetch row")? {
                let mut fields = Vec::with_capacity(names.len());
                for (i, name) in names.iter().enumerate() {
                    let value = row.get_ref(i).context("failed to get column value")?;
                    fields.push(Field {
                        name: name.clone(),
                        value: from_sqlite(value)?,
                    });
                }
                selected.push(Row { fields });
            }

            Ok(selected)
        })
    }

    fn execute(&self, sql: String, params: Vec<Value>) -> FutureResult<u64> {
        tracing::trace!("executing statement: {sql}");
        self.blocking(move |conn| execute(conn, &sql, &params))
    }

    fn batch(&self, statements: Vec<Statement>) -> FutureResult<Vec<u64>> {
        tracing::trace!("executing a batch of {} statements", statements.len());
        self.blocking(move |conn| {
            let tx = conn.transaction().context("failed to begin transaction")?;
            let counts = statements
                .iter()
                .map(|statement| execute(&tx, &statement.sql, &statement.params))
                .collect::<Result<Vec<_>>>()?;
            tx.commit().context("failed to commit transaction")?;
            Ok(counts)
        })
    }
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

fn execute(conn: &SqliteConnection, sql: &str, params: &[Value]) -> Result<u64> {
    let mut stmt = prepare(conn, sql, params)?;
    let changed = stmt.raw_execute().context("failed to execute statement")?;
    Ok(u64::try_from(changed).unwrap_or(u64::MAX))
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
