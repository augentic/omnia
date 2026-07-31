//! Default `SQLite` implementation for wasi-sql
//!
//! This is a lightweight implementation for development use only.

// `derive(FromEnv)` generates undocumented `from_env`/`requirements` associated
// functions that would otherwise trip `missing_docs`.
#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::{Context, Result};
use fromenv::FromEnv;
use futures::FutureExt;
use omnia::Backend;
use rusqlite::types::ValueRef;
use rusqlite::{Connection as SqliteConnection, params_from_iter};
use tracing::instrument;

use crate::host::resource::{Connection, FutureResult};
use crate::host::{DataType, Field, Row, WasiSqlCtx};

/// Options used to connect to the SQL database.
///
/// This struct is used to load connection options from environment variables.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    #[env(from = "SQL_DATABASE", default = "file::memory:?cache=shared")]
    pub database: String,
}

/// Loads connection options from environment variables with error context.
impl omnia::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        // `Self::from_env()` is the builder-returning inherent the `FromEnv`
        // derive emits.
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// Default implementation for `wasi:sql`.
#[derive(Debug, Clone)]
pub struct SqlDefault {
    // Store the database path to create new connections on demand
    // Mutex is necessary since rusqlite::Connection isn't `Sync`
    conn: Arc<parking_lot::Mutex<SqliteConnection>>,
}

impl Backend for SqlDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing SQLite connection to: {}", options.database);

        // Create initial connection to validate database path
        let conn = Arc::new(parking_lot::Mutex::new(
            SqliteConnection::open(&options.database).context("failed to open SQLite database")?,
        ));

        Ok(Self { conn })
    }
}

impl WasiSqlCtx for SqlDefault {
    fn open(&self, _name: String) -> FutureResult<Arc<dyn Connection>> {
        tracing::debug!("opening SQL connection");
        let conn = Arc::clone(&self.conn);

        async move {
            let connection = SqliteConn { conn };
            Ok(Arc::new(connection) as Arc<dyn Connection>)
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct SqliteConn {
    conn: Arc<parking_lot::Mutex<SqliteConnection>>,
}

impl Connection for SqliteConn {
    // The mutex guard must outlive the prepared statement that borrows the
    // connection, so the drop cannot be tightened further.
    #[expect(clippy::significant_drop_tightening)]
    fn query(&self, query: String, params: Vec<DataType>) -> FutureResult<Vec<Row>> {
        tracing::debug!("executing query: {}", query);
        let conn = Arc::clone(&self.conn);

        async move {
            // Blocking rusqlite work (and the mutex held around it) runs on a
            // blocking thread so it never pins an executor thread.
            tokio::task::spawn_blocking(move || {
                let rusqlite_params: Vec<_> = params.iter().map(to_sqlite).collect();

                let conn = conn.lock();
                let mut stmt = conn.prepare(&query).context("failed to prepare statement")?;

                let column_names: Vec<String> =
                    stmt.column_names().iter().map(ToString::to_string).collect();

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

    // See `query`: the guard must outlive the prepared statement.
    #[expect(clippy::significant_drop_tightening)]
    fn exec(&self, query: String, params: Vec<DataType>) -> FutureResult<u32> {
        tracing::debug!("executing statement: {}", query);
        let conn = Arc::clone(&self.conn);

        async move {
            // See `query`: keep the blocking work off the executor.
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

// `u64 as i64` is the standard SQLite convention: store the raw bits and let
// readers reinterpret, since SQLite integers are always signed 64-bit.
#[expect(clippy::cast_possible_wrap)]
fn to_sqlite(dt: &DataType) -> rusqlite::types::Value {
    match dt {
        DataType::Boolean(Some(b)) => rusqlite::types::Value::Integer(i64::from(*b)),
        DataType::Int32(Some(i)) => rusqlite::types::Value::Integer(i64::from(*i)),
        DataType::Int64(Some(i)) => rusqlite::types::Value::Integer(*i),
        DataType::Uint32(Some(u)) => rusqlite::types::Value::Integer(i64::from(*u)),
        DataType::Uint64(Some(u)) => rusqlite::types::Value::Integer(*u as i64),
        DataType::Float(Some(f)) => rusqlite::types::Value::Real(f64::from(*f)),
        DataType::Double(Some(f)) => rusqlite::types::Value::Real(*f),
        DataType::Str(Some(s)) => rusqlite::types::Value::Text(s.clone()),
        DataType::Binary(Some(b)) => rusqlite::types::Value::Blob(b.clone()),
        DataType::Timestamp(Some(ts)) => rusqlite::types::Value::Text(ts.clone()),
        // All None variants map to NULL
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

#[cfg(test)]
mod tests {
    use super::*;

    // The obvious one-to-one mappings mirror the match arms; only the two
    // conventions a reader could get wrong are pinned here.

    #[test]
    fn u64_stores_raw_bits() {
        assert_eq!(
            to_sqlite(&DataType::Uint64(Some(u64::MAX))),
            rusqlite::types::Value::Integer(-1),
        );
    }

    #[test]
    fn invalid_utf8_rejected() {
        from_sqlite(ValueRef::Text(&[0xff])).unwrap_err();
    }
}
