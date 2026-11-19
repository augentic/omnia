//! [`SqlDb`] over `omnia:sql`.

use std::borrow::Cow;

use bytes::Bytes;
use omnia_wasi_sql::omnia::database::{self, Database};
use omnia_wasi_sql::omnia::types::{self, ResultSet};
use omnia_wasi_sql::omnia::{store, transactions};

use crate::sql::{self, DEFAULT, Field, Row, Statement, Value};
use crate::{HasSqlDb, Result, SqlDb, WasiProvider};

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
        let selected = self.open().await?.query(sql.to_owned(), bind(params)).await?;
        read(selected).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Ok(self.open().await?.execute(sql.to_owned(), bind(params)).await?)
    }

    async fn batch(&self, statements: &[Statement]) -> Result<Vec<u64>> {
        let statements = statements
            .iter()
            .map(|statement| database::Statement {
                sql: statement.sql.clone(),
                params: bind(&statement.params),
            })
            .collect();
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

impl sql::Transaction for Transaction {
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        read(self.inner.query(sql.to_owned(), bind(params)).await?).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Ok(self.inner.execute(sql.to_owned(), bind(params)).await?)
    }

    async fn commit(self) -> Result<()> {
        Ok(self.inner.commit().await?)
    }

    async fn rollback(self) -> Result<()> {
        Ok(self.inner.rollback().await?)
    }
}

fn bind(params: &[Value]) -> Vec<Option<types::Value>> {
    params.iter().map(wit_value).collect()
}

fn wit_value(value: &Value) -> Option<types::Value> {
    let value = match value.clone() {
        Value::Null => return None,
        Value::Int32(v) => types::Value::Int32(v),
        Value::Int64(v) => types::Value::Int64(v),
        Value::Float(v) => types::Value::Float32(v),
        Value::Double(v) => types::Value::Float64(v),
        Value::Str(v) => types::Value::Text(v),
        Value::Boolean(v) => types::Value::Boolean(v),
        Value::Date(v) => types::Value::Date(v),
        Value::Time(v) => types::Value::Time(v),
        Value::Timestamp(v) if has_offset(&v) => types::Value::TimestampTz(v),
        Value::Timestamp(v) => types::Value::Timestamp(v),
        Value::Binary(v) => types::Value::Binary(v.to_vec()),
    };
    Some(value)
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
                    value: value.map_or(Value::Null, Value::from),
                })
                .collect(),
        })
        .collect())
}

impl From<types::Value> for Value {
    fn from(value: types::Value) -> Self {
        match value {
            types::Value::Boolean(v) => Self::Boolean(v),
            types::Value::Int32(v) => Self::Int32(v),
            types::Value::Int64(v) => Self::Int64(v),
            types::Value::Float32(v) => Self::Float(v),
            types::Value::Float64(v) => Self::Double(v),
            types::Value::Text(v) => Self::Str(v),
            types::Value::Binary(v) => Self::Binary(Bytes::from(v)),
            types::Value::Date(v) => Self::Date(v),
            types::Value::Time(v) => Self::Time(v),
            types::Value::Timestamp(v) | types::Value::TimestampTz(v) => Self::Timestamp(v),
        }
    }
}
