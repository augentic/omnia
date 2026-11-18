//! [`SqlDb`] over `omnia:sql`.

use std::borrow::Cow;

use bytes::Bytes;
use omnia_wasi_sql::omnia::database::{self, Database};
use omnia_wasi_sql::omnia::types::{self, ResultSet};
use omnia_wasi_sql::omnia::{store, transactions};

use crate::sql::{self, DEFAULT, Field, Row, Statement, Value};
use crate::{Error, HasSqlDb, Result, SqlDb, WasiProvider};

static DEFAULT_HANDLE: Handle = Handle {
    name: Cow::Borrowed(DEFAULT),
};

impl HasSqlDb for WasiProvider {
    type Inner = Handle;

    fn sql_db(&self) -> &Handle {
        &DEFAULT_HANDLE
    }
}

/// A [`SqlDb`] handle on one database of the WASI host.
///
/// [`SqlDb::named`] on [`WasiProvider`] returns one.
#[derive(Clone, Debug)]
pub struct Handle {
    name: Cow<'static, str>,
}

impl Handle {
    async fn open(&self) -> Result<Database> {
        Ok(store::open(self.name.to_string()).await?)
    }
}

impl SqlDb for Handle {
    type Named = Self;
    type Transaction = Transaction;

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let params = bind(params)?;
        let selected = self.open().await?.query(sql.to_owned(), params).await?;
        read(selected).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let params = bind(params)?;
        Ok(self.open().await?.execute(sql.to_owned(), params).await?)
    }

    async fn batch(&self, statements: &[Statement]) -> Result<Vec<u64>> {
        let statements = statements
            .iter()
            .map(|statement| {
                Ok(database::Statement {
                    sql: statement.sql.clone(),
                    params: bind(&statement.params)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(self.open().await?.batch(statements).await?)
    }

    async fn begin(&self) -> Result<Transaction> {
        let database = self.open().await?;
        let inner = transactions::begin(&database).await?;
        Ok(Transaction { inner })
    }

    fn named(&self, name: &str) -> Self {
        Self {
            name: Cow::Owned(name.to_owned()),
        }
    }
}

/// A [`Transaction`](sql::Transaction) on one database of the WASI host.
///
/// [`SqlDb::begin`] on a [`Handle`] returns one. Dropping it ends the host's
/// transaction, which rolls back.
#[derive(Debug)]
pub struct Transaction {
    inner: transactions::Transaction,
}

impl Transaction {
    // A value that does not bind ends the transaction, as a failure at the
    // host does.
    async fn bound(&self, params: &[Value]) -> Result<Vec<Option<types::Value>>> {
        let bound = bind(params);
        if bound.is_err() {
            drop(self.inner.rollback().await);
        }
        bound
    }
}

impl sql::Transaction for Transaction {
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let params = self.bound(params).await?;
        read(self.inner.query(sql.to_owned(), params).await?).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let params = self.bound(params).await?;
        Ok(self.inner.execute(sql.to_owned(), params).await?)
    }

    async fn commit(self) -> Result<()> {
        Ok(self.inner.commit().await?)
    }

    async fn rollback(self) -> Result<()> {
        Ok(self.inner.rollback().await?)
    }
}

fn bind(params: &[Value]) -> Result<Vec<Option<types::Value>>> {
    params.iter().zip(1..).map(|(value, n)| wit_value(value, n)).collect()
}

fn wit_value(value: &Value, n: usize) -> Result<Option<types::Value>> {
    Ok(match value.clone() {
        Value::Int32(v) => v.map(types::Value::Int32),
        Value::Int64(v) => v.map(types::Value::Int64),
        Value::Uint32(v) => v.map(|v| types::Value::Int64(i64::from(v))),
        Value::Uint64(v) => v
            .map(|v| {
                i64::try_from(v).map(types::Value::Int64).map_err(|_overflow| {
                    Error::server_error("invalid", format!("parameter `${n}` is past `i64::MAX`"))
                })
            })
            .transpose()?,
        Value::Float(v) => v.map(types::Value::Float32),
        Value::Double(v) => v.map(types::Value::Float64),
        Value::Str(v) => v.map(types::Value::Text),
        Value::Boolean(v) => v.map(types::Value::Boolean),
        Value::Date(v) => v.map(types::Value::Date),
        Value::Time(v) => v.map(types::Value::Time),
        Value::Timestamp(v) => v.map(|v| {
            if has_offset(&v) { types::Value::TimestampTz(v) } else { types::Value::Timestamp(v) }
        }),
        Value::Binary(v) => v.map(|bytes| types::Value::Binary(bytes.to_vec())),
    })
}

// An RFC 3339 offset follows the time, past the `YYYY-MM-DD` date.
fn has_offset(timestamp: &str) -> bool {
    let time = timestamp.get(10..).unwrap_or_default();
    time.ends_with(['Z', 'z']) || time.contains(['+', '-'])
}

async fn read(selected: ResultSet) -> Result<Vec<Row>> {
    let ResultSet {
        columns,
        rows,
        finished,
    } = selected;
    let rows = rows.collect().await;
    finished.await?;

    Ok(rows
        .into_iter()
        .map(|values| Row {
            fields: columns
                .iter()
                .zip(values)
                .map(|(name, value)| Field {
                    name: name.clone(),
                    value: value.map_or(Value::Str(None), Value::from),
                })
                .collect(),
        })
        .collect())
}

impl From<types::Value> for Value {
    fn from(value: types::Value) -> Self {
        match value {
            types::Value::Boolean(v) => Self::Boolean(Some(v)),
            types::Value::Int32(v) => Self::Int32(Some(v)),
            types::Value::Int64(v) => Self::Int64(Some(v)),
            types::Value::Float32(v) => Self::Float(Some(v)),
            types::Value::Float64(v) => Self::Double(Some(v)),
            types::Value::Text(v) => Self::Str(Some(v)),
            types::Value::Binary(v) => Self::Binary(Some(Bytes::from(v))),
            types::Value::Date(v) => Self::Date(Some(v)),
            types::Value::Time(v) => Self::Time(Some(v)),
            types::Value::Timestamp(v) | types::Value::TimestampTz(v) => Self::Timestamp(Some(v)),
        }
    }
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(message) => Self::server_error("not-found", message),
            types::Error::Denied(message) => Self::server_error("denied", message),
            types::Error::Invalid(message) => Self::server_error("invalid", message),
            types::Error::Unsupported(message) => Self::server_error("unsupported", message),
            types::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}
