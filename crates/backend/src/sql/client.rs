//! Service contract for `wasi:sql` backends.

use std::fmt::Debug;
use std::sync::Arc;

use super::{DataType, Row};
use crate::FutureResult;

/// Service contract for `wasi:sql` backends.
pub trait SqlClient: Debug + Send + Sync + 'static {
    /// Open a connection to the database `name`.
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>>;
}

/// Stateful handle to a SQL database connection.
pub trait Connection: Debug + Send + Sync + 'static {
    /// Execute a query and return the resulting rows.
    fn query(&self, query: String, params: Vec<DataType>) -> FutureResult<Vec<Row>>;

    /// Execute a statement that does not return rows (e.g., `INSERT`, `UPDATE`, `DELETE`).
    fn exec(&self, query: String, params: Vec<DataType>) -> FutureResult<u32>;
}
