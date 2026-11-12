//! The `wasi:sql` handle and the conversions between the SQL values and the bindings.

use std::future::{Future, ready};

use bytes::Bytes;
use omnia_wasi_sql::readwrite;
use omnia_wasi_sql::types::{self, Connection};

use super::{Field, Row, Statement, Value};
use crate::SqlDb;
use crate::capability::{Error, ErrorKind, Result};

/// A [`SqlDb`](crate::SqlDb) handle on one database of the WASI host.
///
/// [`SqlDb::named`](crate::SqlDb::named) on [`WasiProvider`](crate::WasiProvider)
/// returns one. `wasi:sql` has no transactions, so
/// [`batch`](crate::SqlDb::batch) fails with
/// [`ErrorKind::Unsupported`](crate::capability::ErrorKind::Unsupported).
#[derive(Clone, Debug)]
pub struct Handle {
    name: String,
}

impl Handle {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
        }
    }

    async fn prepare(&self, sql: &str, params: &[Value]) -> Result<(Connection, types::Statement)> {
        let connection = Connection::open(self.name.clone()).await?;
        let params = params.iter().cloned().map(Into::into).collect();
        let statement = types::Statement::prepare(sql.to_owned(), params).await?;
        Ok((connection, statement))
    }
}

impl SqlDb for Handle {
    type Named = Self;

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let (connection, statement) = self.prepare(sql, params).await?;
        let rows = readwrite::query(&connection, &statement).await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let (connection, statement) = self.prepare(sql, params).await?;
        Ok(readwrite::exec(&connection, &statement).await?.into())
    }

    fn batch(&self, _statements: &[Statement]) -> impl Future<Output = Result<Vec<u64>>> + Send {
        let message = "`wasi:sql` cannot run a batch in one transaction";
        ready(Err(Error::new(ErrorKind::Unsupported, message)))
    }

    fn named(&self, name: &str) -> Self {
        Self::new(name)
    }
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        Self::new(ErrorKind::Other, err.trace())
    }
}

impl From<Value> for types::DataType {
    fn from(value: Value) -> Self {
        match value {
            Value::Int32(v) => Self::Int32(v),
            Value::Int64(v) => Self::Int64(v),
            Value::Uint32(v) => Self::Uint32(v),
            Value::Uint64(v) => Self::Uint64(v),
            Value::Float(v) => Self::Float(v),
            Value::Double(v) => Self::Double(v),
            Value::Str(v) => Self::Str(v),
            Value::Boolean(v) => Self::Boolean(v),
            Value::Date(v) => Self::Date(v),
            Value::Time(v) => Self::Time(v),
            Value::Timestamp(v) => Self::Timestamp(v),
            Value::Binary(v) => Self::Binary(v.map(|bytes| bytes.to_vec())),
        }
    }
}

impl From<types::DataType> for Value {
    fn from(value: types::DataType) -> Self {
        match value {
            types::DataType::Int32(v) => Self::Int32(v),
            types::DataType::Int64(v) => Self::Int64(v),
            types::DataType::Uint32(v) => Self::Uint32(v),
            types::DataType::Uint64(v) => Self::Uint64(v),
            types::DataType::Float(v) => Self::Float(v),
            types::DataType::Double(v) => Self::Double(v),
            types::DataType::Str(v) => Self::Str(v),
            types::DataType::Boolean(v) => Self::Boolean(v),
            types::DataType::Date(v) => Self::Date(v),
            types::DataType::Time(v) => Self::Time(v),
            types::DataType::Timestamp(v) => Self::Timestamp(v),
            types::DataType::Binary(v) => Self::Binary(v.map(Bytes::from)),
        }
    }
}

impl From<types::Field> for Field {
    fn from(field: types::Field) -> Self {
        Self {
            name: field.name,
            value: field.value.into(),
        }
    }
}

impl From<types::Row> for Row {
    fn from(row: types::Row) -> Self {
        Self {
            fields: row.fields.into_iter().map(Into::into).collect(),
        }
    }
}
