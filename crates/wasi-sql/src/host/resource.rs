use std::fmt::Debug;

pub use omnia_core::FutureResult;

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

/// Proxy for a SQL connection, stored in the resource table.
pub type ConnectionProxy = omnia_core::Proxy<dyn Connection>;

/// Represents a statement resource in the WASI SQL host.
#[derive(Clone, Debug)]
pub struct Statement {
    /// SQL query string.
    pub query: String,

    /// Query parameters.
    pub params: Vec<DataType>,
}
