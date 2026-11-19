//! The connection a backend serves, and the values it exchanges with the host.

use std::fmt::Debug;

pub use omnia_core::FutureResult;

/// A connection to one database, which runs the statements a guest sends.
///
/// [`SqlClient::open`](crate::SqlClient::open) returns one. A statement
/// numbers its parameters `$1`, `$2`, and so on, and each binds the value at
/// that position, in any order and as often as it appears.
///
/// A backend picks the kind of a failure by returning an
/// [`Error`](crate::Error), and any other error reaches the guest as
/// [`Error::Other`](crate::Error::Other).
///
/// The host runs a transaction's statements on one connection, between
/// [`begin`](Self::begin) and [`commit`](Self::commit) or
/// [`rollback`](Self::rollback). Their default bodies send `BEGIN`, `COMMIT`,
/// and `ROLLBACK` through [`execute`](Self::execute), which is correct when a
/// connection is one session of its own.
///
/// - A backend whose connections share a session overrides all three.
/// - A backend that keeps no session between calls answers
///   [`Error::Unsupported`](crate::Error::Unsupported) from `begin`.
pub trait Connection: Debug + Send + Sync + 'static {
    /// Run `sql` with `params` bound and return the rows it selects.
    fn query(&self, sql: String, params: Vec<Value>) -> FutureResult<Vec<Row>>;

    /// Run `sql` with `params` bound and return the number of rows it changed.
    fn execute(&self, sql: String, params: Vec<Value>) -> FutureResult<u64>;

    /// Run `statements` in order, in one transaction, and return each one's
    /// changed-row count.
    ///
    /// A failing statement rolls back the whole batch, and the call fails
    /// with that statement's error.
    fn batch(&self, statements: Vec<Statement>) -> FutureResult<Vec<u64>>;

    /// Run `sql` with `params` bound and return its columns and the rows it
    /// selects.
    ///
    /// The default body runs [`query`](Self::query) and takes the column
    /// names from the first row, so it names no columns when no row is
    /// selected.
    fn select(&self, sql: String, params: Vec<Value>) -> FutureResult<ResultSet> {
        let rows = self.query(sql, params);
        Box::pin(async move {
            let rows = rows.await?;
            let columns = rows
                .first()
                .map_or_default(|row| row.fields.iter().map(|field| field.name.clone()).collect());
            Ok(ResultSet { columns, rows })
        })
    }

    /// Begin a transaction on this connection.
    fn begin(&self) -> FutureResult<()> {
        unit(self.execute("BEGIN".to_owned(), Vec::new()))
    }

    /// Commit the transaction [`begin`](Self::begin) started.
    fn commit(&self) -> FutureResult<()> {
        unit(self.execute("COMMIT".to_owned(), Vec::new()))
    }

    /// Roll back the transaction [`begin`](Self::begin) started.
    ///
    /// The host also calls it after any failure in the transaction, including
    /// a failed [`commit`](Self::commit), when the backend may have ended the
    /// transaction already.
    fn rollback(&self) -> FutureResult<()> {
        unit(self.execute("ROLLBACK".to_owned(), Vec::new()))
    }
}

fn unit(executed: FutureResult<u64>) -> FutureResult<()> {
    Box::pin(async move { executed.await.map(|_| ()) })
}

/// A value bound to a parameter or read from a column, where `None` is
/// `NULL`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A signed 32-bit integer.
    Int32(Option<i32>),
    /// A signed 64-bit integer.
    Int64(Option<i64>),
    /// An unsigned 32-bit integer.
    Uint32(Option<u32>),
    /// An unsigned 64-bit integer.
    Uint64(Option<u64>),
    /// A 32-bit floating-point number.
    Float(Option<f32>),
    /// A 64-bit floating-point number.
    Double(Option<f64>),
    /// A UTF-8 string.
    Str(Option<String>),
    /// A boolean.
    Boolean(Option<bool>),
    /// A `YYYY-MM-DD` date.
    Date(Option<String>),
    /// An `HH:MM:SS` time, with optional fractional seconds.
    Time(Option<String>),
    /// A `YYYY-MM-DD HH:MM:SS` timestamp, or RFC 3339 for a `TIMESTAMPTZ`
    /// column.
    Timestamp(Option<String>),
    /// Opaque bytes.
    Binary(Option<Vec<u8>>),
}

/// One column of a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The column name.
    pub name: String,
    /// The column value.
    pub value: Value,
}

/// One row a query selects.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The columns, in select order.
    pub fields: Vec<Field>,
}

/// One statement of a batch.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    /// The SQL text.
    pub sql: String,
    /// The values its `$n` parameters bind.
    pub params: Vec<Value>,
}

/// The columns a query names and the rows it selects.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultSet {
    /// The name of each column, in select order.
    pub columns: Vec<String>,
    /// The rows, each holding one field for every column.
    pub rows: Vec<Row>,
}
