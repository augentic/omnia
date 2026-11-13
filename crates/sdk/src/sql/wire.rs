//! The `omnia:sql` handle and the conversions between the SQL values and the bindings.

use bytes::Bytes;
use omnia_wasi_sql::omnia::database;

use super::{Field, Row, Statement, Value};
use crate::SqlDb;
use crate::capability::{Error, ErrorKind, Result};

/// A [`SqlDb`](crate::SqlDb) handle on one database of the WASI host.
///
/// [`SqlDb::named`](crate::SqlDb::named) on [`WasiProvider`](crate::WasiProvider)
/// returns one.
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
}

impl SqlDb for Handle {
    type Named = Self;

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let params = params.iter().cloned().map(Into::into).collect();
        let rows = database::query(self.name.clone(), sql.to_owned(), params).await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let params = params.iter().cloned().map(Into::into).collect();
        Ok(database::execute(self.name.clone(), sql.to_owned(), params).await?)
    }

    async fn batch(&self, statements: &[Statement]) -> Result<Vec<u64>> {
        let statements = statements.iter().cloned().map(Into::into).collect();
        Ok(database::batch(self.name.clone(), statements).await?)
    }

    fn named(&self, name: &str) -> Self {
        Self::new(name)
    }
}

impl From<database::Error> for Error {
    fn from(err: database::Error) -> Self {
        let (kind, message) = match err {
            database::Error::NotFound(message) => (ErrorKind::NotFound, message),
            database::Error::Denied(message) => (ErrorKind::Denied, message),
            database::Error::Invalid(message) => (ErrorKind::Invalid, message),
            database::Error::Unsupported(message) => (ErrorKind::Unsupported, message),
            database::Error::Unavailable(message) => (ErrorKind::Unavailable, message),
            database::Error::Other(message) => (ErrorKind::Other, message),
        };
        Self::new(kind, message)
    }
}

impl From<Statement> for database::Statement {
    fn from(statement: Statement) -> Self {
        Self {
            sql: statement.sql,
            params: statement.params.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Value> for database::Value {
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

impl From<database::Value> for Value {
    fn from(value: database::Value) -> Self {
        match value {
            database::Value::Int32(v) => Self::Int32(v),
            database::Value::Int64(v) => Self::Int64(v),
            database::Value::Uint32(v) => Self::Uint32(v),
            database::Value::Uint64(v) => Self::Uint64(v),
            database::Value::Float(v) => Self::Float(v),
            database::Value::Double(v) => Self::Double(v),
            database::Value::Str(v) => Self::Str(v),
            database::Value::Boolean(v) => Self::Boolean(v),
            database::Value::Date(v) => Self::Date(v),
            database::Value::Time(v) => Self::Time(v),
            database::Value::Timestamp(v) => Self::Timestamp(v),
            database::Value::Binary(v) => Self::Binary(v.map(Bytes::from)),
        }
    }
}

impl From<database::Field> for Field {
    fn from(field: database::Field) -> Self {
        Self {
            name: field.name,
            value: field.value.into(),
        }
    }
}

impl From<database::Row> for Row {
    fn from(row: database::Row) -> Self {
        Self {
            fields: row.fields.into_iter().map(Into::into).collect(),
        }
    }
}
