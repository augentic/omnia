use std::fmt::Debug;

pub use omnia_core::FutureResult;

use crate::host::{Row, Statement, Value};

/// A connection to one database, which runs the statements a guest sends.
///
/// [`SqlClient::open`](crate::SqlClient::open) returns one. A statement
/// numbers its parameters `$1`, `$2`, and so on, and each binds the value at
/// that position, in any order and as often as it appears.
///
/// A backend picks the kind of a failure by returning an
/// [`Error`](crate::Error), and any other error reaches the guest as
/// [`Error::Other`](crate::Error::Other).
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
}

/// Proxy for a SQL connection, stored in the resource table.
pub type ConnectionProxy = omnia_core::Proxy<dyn Connection>;

// the draft's `statement` resource
#[derive(Clone, Debug)]
pub struct Prepared {
    pub sql: String,
    pub params: Vec<Value>,
}

// the draft's `error` resource, which renders a failure only as text
#[derive(Debug)]
pub struct Trace(pub String);
