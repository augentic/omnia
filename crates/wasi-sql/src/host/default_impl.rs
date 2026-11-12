//! Default `SQLite` implementation for wasi-sql
//!
//! This is a lightweight implementation for development use only.

use std::collections::{BTreeMap, HashMap};
use std::env::{self, VarError};
use std::iter;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use futures::FutureExt;
use omnia_core::Connect;
use rusqlite::types::ValueRef;
use rusqlite::{Connection as SqliteConnection, params_from_iter};
use tracing::instrument;

use crate::host::resource::{Connection, FutureResult};
use crate::host::{DataType, Field, Row, SqlClient};

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

/// Default implementation for `wasi:sql`.
///
/// It holds one connection per declared database and refuses to open any
/// other name. Names match case-insensitively.
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
            let conn = conn.with_context(|| format!("database `{name}` is not declared"))?;
            Ok(Arc::new(SqliteConn { conn }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct SqliteConn {
    conn: Arc<parking_lot::Mutex<SqliteConnection>>,
}

impl Connection for SqliteConn {
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the prepared statement borrows the connection the guard holds"
    )]
    fn query(&self, query: String, params: Vec<DataType>) -> FutureResult<Vec<Row>> {
        tracing::trace!("executing query: {}", query);
        let conn = Arc::clone(&self.conn);

        async move {
            // rusqlite blocks, and holds the mutex, so it never pins an executor thread
            tokio::task::spawn_blocking(move || {
                let rusqlite_params: Vec<_> = params.iter().map(to_sqlite).collect();

                let conn = conn.lock();
                let mut stmt = conn.prepare(&query).context("failed to prepare statement")?;

                let column_names: Vec<String> =
                    stmt.column_names().map(ToString::to_string).collect();

                let mut rows = stmt
                    .query(params_from_iter(rusqlite_params.iter()))
                    .context("failed to execute query")?;

                let mut result_rows = Vec::new();
                let mut index = 0;
                while let Some(row) = rows.next().context("failed to fetch row")? {
                    let mut fields = Vec::new();

                    for (i, name) in column_names.iter().enumerate() {
                        let value = row.get_ref(i).context("failed to get column value")?;
                        let data_type = from_sqlite(value)?;

                        fields.push(Field {
                            name: name.clone(),
                            value: data_type,
                        });
                    }

                    result_rows.push(Row {
                        index: index.to_string(),
                        fields,
                    });
                    index += 1;
                }

                Ok(result_rows)
            })
            .await
            .context("query task panicked")?
        }
        .boxed()
    }

    #[expect(
        clippy::significant_drop_tightening,
        reason = "the prepared statement borrows the connection the guard holds"
    )]
    fn exec(&self, query: String, params: Vec<DataType>) -> FutureResult<u32> {
        tracing::trace!("executing statement: {}", query);
        let conn = Arc::clone(&self.conn);

        async move {
            // as `query`: keep the blocking work off the executor
            tokio::task::spawn_blocking(move || {
                let rusqlite_params: Vec<_> = params.iter().map(to_sqlite).collect();

                let conn = conn.lock();
                let mut stmt = conn.prepare(&query).context("failed to prepare statement")?;

                let rows_affected = stmt
                    .execute(params_from_iter(rusqlite_params.iter()))
                    .context("failed to execute statement")?;

                Ok(u32::try_from(rows_affected).unwrap_or(u32::MAX))
            })
            .await
            .context("exec task panicked")?
        }
        .boxed()
    }
}

// A `u64` is stored as its raw bits, the `SQLite` convention: its integers
// are always signed 64-bit, and readers reinterpret.
fn to_sqlite(dt: &DataType) -> rusqlite::types::Value {
    match dt {
        DataType::Boolean(Some(b)) => rusqlite::types::Value::Integer(i64::from(*b)),
        DataType::Int32(Some(i)) => rusqlite::types::Value::Integer(i64::from(*i)),
        DataType::Int64(Some(i)) => rusqlite::types::Value::Integer(*i),
        DataType::Uint32(Some(u)) => rusqlite::types::Value::Integer(i64::from(*u)),
        DataType::Uint64(Some(u)) => rusqlite::types::Value::Integer(u.cast_signed()),
        DataType::Float(Some(f)) => rusqlite::types::Value::Real(f64::from(*f)),
        DataType::Double(Some(f)) => rusqlite::types::Value::Real(*f),
        DataType::Str(Some(s)) => rusqlite::types::Value::Text(s.clone()),
        DataType::Binary(Some(b)) => rusqlite::types::Value::Blob(b.clone()),
        DataType::Timestamp(Some(ts)) => rusqlite::types::Value::Text(ts.clone()),
        // every `None` variant
        _ => rusqlite::types::Value::Null,
    }
}

fn from_sqlite(value: ValueRef) -> Result<DataType> {
    match value {
        ValueRef::Null => Ok(DataType::Str(None)),
        ValueRef::Integer(i) => Ok(DataType::Int64(Some(i))),
        ValueRef::Real(f) => Ok(DataType::Double(Some(f))),
        ValueRef::Text(t) => {
            let s = std::str::from_utf8(t).context("invalid UTF-8 in text value")?;
            Ok(DataType::Str(Some(s.to_string())))
        }
        ValueRef::Blob(b) => Ok(DataType::Binary(Some(b.to_vec()))),
    }
}
