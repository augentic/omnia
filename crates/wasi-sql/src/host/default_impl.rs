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
    fn from_env() -> Result<Self> {
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
            let connection = SqliteConnectionImpl { conn };
            Ok(Arc::new(connection) as Arc<dyn Connection>)
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct SqliteConnectionImpl {
    conn: Arc<parking_lot::Mutex<SqliteConnection>>,
}

impl Connection for SqliteConnectionImpl {
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
                let rusqlite_params: Vec<_> =
                    params.iter().map(datatype_to_rusqlite_value).collect();

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
                        let data_type = rusqlite_value_to_datatype(value)?;

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
                let rusqlite_params: Vec<_> =
                    params.iter().map(datatype_to_rusqlite_value).collect();

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
fn datatype_to_rusqlite_value(dt: &DataType) -> rusqlite::types::Value {
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

fn rusqlite_value_to_datatype(value: ValueRef) -> Result<DataType> {
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

    #[test]
    fn datatype_to_sqlite_values() {
        use rusqlite::types::Value;

        assert_eq!(datatype_to_rusqlite_value(&DataType::Boolean(Some(true))), Value::Integer(1));
        assert_eq!(datatype_to_rusqlite_value(&DataType::Int32(Some(-7))), Value::Integer(-7));
        assert_eq!(datatype_to_rusqlite_value(&DataType::Int64(Some(9))), Value::Integer(9));
        assert_eq!(datatype_to_rusqlite_value(&DataType::Uint32(Some(4))), Value::Integer(4));
        assert_eq!(
            datatype_to_rusqlite_value(&DataType::Uint64(Some(u64::MAX))),
            Value::Integer(-1),
            "u64 stores its raw bits"
        );
        assert_eq!(datatype_to_rusqlite_value(&DataType::Float(Some(1.5))), Value::Real(1.5));
        assert_eq!(datatype_to_rusqlite_value(&DataType::Double(Some(2.5))), Value::Real(2.5));
        assert_eq!(
            datatype_to_rusqlite_value(&DataType::Str(Some("s".to_string()))),
            Value::Text("s".to_string())
        );
        assert_eq!(
            datatype_to_rusqlite_value(&DataType::Binary(Some(vec![1, 2]))),
            Value::Blob(vec![1, 2])
        );
        assert_eq!(
            datatype_to_rusqlite_value(&DataType::Timestamp(Some("2026-01-01".to_string()))),
            Value::Text("2026-01-01".to_string())
        );
        assert_eq!(datatype_to_rusqlite_value(&DataType::Str(None)), Value::Null);
    }

    #[test]
    fn sqlite_value_to_datatypes() {
        assert!(matches!(
            rusqlite_value_to_datatype(ValueRef::Null).expect("null"),
            DataType::Str(None)
        ));
        assert!(matches!(
            rusqlite_value_to_datatype(ValueRef::Integer(3)).expect("integer"),
            DataType::Int64(Some(3))
        ));
        assert!(matches!(
            rusqlite_value_to_datatype(ValueRef::Real(0.5)).expect("real"),
            DataType::Double(Some(v)) if (v - 0.5).abs() < f64::EPSILON
        ));
        assert!(matches!(
            rusqlite_value_to_datatype(ValueRef::Text(b"t")).expect("text"),
            DataType::Str(Some(s)) if s == "t"
        ));
        assert!(matches!(
            rusqlite_value_to_datatype(ValueRef::Blob(&[7])).expect("blob"),
            DataType::Binary(Some(b)) if b == vec![7]
        ));
        assert!(rusqlite_value_to_datatype(ValueRef::Text(&[0xff])).is_err(), "invalid UTF-8");
    }
}
