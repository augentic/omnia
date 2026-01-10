use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

pub use yetti::FutureResult;

use crate::host::{DataType, Row};

/// SQL providers implement the [`Connection`] trait to allow the host to
/// connect to a backend (Azure Table Storage, Postgres, etc) and execute SQL
/// statements.
pub trait Connection: Debug + Send + Sync + 'static {
    /// Execute a query and return the resulting rows.
    fn query(&self, query: String, params: Vec<DataType>) -> FutureResult<Vec<Row>>;

    /// Execute a query that does not return rows (e.g., an `INSERT`, `UPDATE`, or `DELETE`).
    fn exec(&self, query: String, params: Vec<DataType>) -> FutureResult<u32>;
}

/// [`ConnectionProxy`] provides a concrete wrapper around a `dyn Connection` object.
/// It is used to store connection resources in the resource table.
#[derive(Clone, Debug)]
pub struct ConnectionProxy(pub Arc<dyn Connection>);

impl Deref for ConnectionProxy {
    type Target = Arc<dyn Connection>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Represents a statement resource in the WASI SQL host.
#[derive(Clone, Debug)]
pub struct Statement {
    /// SQL query string.
    pub query: String,

    /// Query parameters.
    pub params: Vec<DataType>,
}
