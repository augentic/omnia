//! Default `SQLite` implementation for wasi-sql
//!
//! This is a lightweight implementation for development use only.

#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_lossless)]
#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::{Context, Result};
use fromenv::FromEnv;
use futures::FutureExt;
use qwasr::Backend;
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

#[allow(missing_docs)]
impl qwasr::FromEnv for ConnectOptions {
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
    fn query(&self, query: String, params: Vec<DataType>) -> FutureResult<Vec<Row>> {
        tracing::debug!("executing query: {}", query);
        let conn = Arc::clone(&self.conn);

        async move {
            let conn = conn.lock();
            let mut stmt = conn.prepare(&query).context("failed to prepare statement")?;

            // Convert DataType to rusqlite values
            let rusqlite_params: Vec<_> = params.iter().map(datatype_to_rusqlite_value).collect();

            // Get column names
            let column_names: Vec<String> =
                stmt.column_names().iter().map(ToString::to_string).collect();

            // Execute query and collect rows
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
        }
        .boxed()
    }

    fn exec(&self, query: String, params: Vec<DataType>) -> FutureResult<u32> {
        tracing::debug!("executing statement: {}", query);
        let conn = Arc::clone(&self.conn);

        async move {
            let conn = conn.lock();
            let mut stmt = conn.prepare(&query).context("failed to prepare statement")?;

            // Convert DataType to rusqlite values
            let rusqlite_params: Vec<_> = params.iter().map(datatype_to_rusqlite_value).collect();

            let rows_affected = stmt
                .execute(params_from_iter(rusqlite_params.iter()))
                .context("failed to execute statement")?;

            #[allow(clippy::cast_possible_truncation)]
            Ok(rows_affected as u32)
        }
        .boxed()
    }
}

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

    #[tokio::test]
    async fn sqlite_operations() {
        let ctx = SqlDefault::connect_with(ConnectOptions {
            database: ":memory:".to_string(),
        })
        .await
        .expect("connect");

        let conn = ctx.open("test".to_string()).await.expect("open connection");

        // Create a test table
        let rows_affected = conn
            .exec(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)".to_string(),
                vec![],
            )
            .await
            .expect("create table");
        assert_eq!(rows_affected, 0);

        // Insert some data
        let rows_affected = conn
            .exec(
                "INSERT INTO users (name, age) VALUES (?, ?)".to_string(),
                vec![DataType::Str(Some("Alice".to_string())), DataType::Int32(Some(30))],
            )
            .await
            .expect("insert");
        assert_eq!(rows_affected, 1);

        let rows_affected = conn
            .exec(
                "INSERT INTO users (name, age) VALUES (?, ?)".to_string(),
                vec![DataType::Str(Some("Bob".to_string())), DataType::Int32(Some(25))],
            )
            .await
            .expect("insert");
        assert_eq!(rows_affected, 1);

        // Query the data
        let rows = conn
            .query("SELECT id, name, age FROM users ORDER BY name".to_string(), vec![])
            .await
            .expect("query");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].fields[1].name, "name");
        if let DataType::Str(Some(ref name)) = rows[0].fields[1].value {
            assert_eq!(name, "Alice");
        } else {
            panic!("Expected string value");
        }
    }
}
