//! Relational (SQL) table capability backing the ORM.

use std::future::Future;

use anyhow::Result;
use omnia_wasi_sql::{DataType, Row};

/// Types that provide ORM database access.
///
/// Default WASM implementations use the WASI SQL bindings to execute queries.
pub trait TableStore: Send + Sync {
    /// Executes a query and returns the result rows.
    #[cfg(not(target_arch = "wasm32"))]
    fn query(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send;

    /// Executes a statement and returns the number of affected rows.
    #[cfg(not(target_arch = "wasm32"))]
    fn exec(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<u32>> + Send;

    /// Executes a query and returns the result rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or query execution fails.
    #[cfg(target_arch = "wasm32")]
    fn query(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send {
        async move {
            let (conn, stmt) = prepare(conn_name, query, params).await?;
            omnia_wasi_sql::readwrite::query(&conn, &stmt)
                .await
                .map_err(|e| anyhow::anyhow!("query failed: {}", e.trace()))
        }
    }

    /// Executes a statement and returns the number of affected rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or execution fails.
    #[cfg(target_arch = "wasm32")]
    fn exec(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<u32>> + Send {
        async move {
            let (conn, stmt) = prepare(conn_name, query, params).await?;
            omnia_wasi_sql::readwrite::exec(&conn, &stmt)
                .await
                .map_err(|e| anyhow::anyhow!("exec failed: {}", e.trace()))
        }
    }
}

/// Open the connection and prepare the statement shared by `query` and `exec`.
#[cfg(target_arch = "wasm32")]
async fn prepare(
    conn_name: String, query: String, params: Vec<DataType>,
) -> Result<(omnia_wasi_sql::types::Connection, omnia_wasi_sql::types::Statement)> {
    use omnia_wasi_sql::types::{Connection, Statement};

    let conn = Connection::open(conn_name)
        .await
        .map_err(|e| anyhow::anyhow!("failed to open connection: {}", e.trace()))?;
    let stmt = Statement::prepare(query, params)
        .await
        .map_err(|e| anyhow::anyhow!("failed to prepare statement: {}", e.trace()))?;
    Ok((conn, stmt))
}
