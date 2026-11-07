//! Relational (SQL) table capability over `wasi:sql`.

use std::future::Future;

use anyhow::Result;
use omnia_backend::sql::{DataType, FromRow, Row};

/// Types that execute parameterized SQL against a named connection.
///
/// [`WasiProvider`](crate::WasiProvider) uses the WASI SQL bindings to execute
/// queries. Typed row decoding is [`TableStoreExt`].
#[omnia_guest_macros::capability]
pub trait TableStore: Send + Sync {
    /// Executes a query and returns the result rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or query execution fails.
    fn query(
        &self, conn_name: &str, query: &str, params: &[DataType],
    ) -> impl Future<Output = Result<Vec<Row>>> + Send;

    /// Executes a statement and returns the number of affected rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or execution fails.
    fn exec(
        &self, conn_name: &str, query: &str, params: &[DataType],
    ) -> impl Future<Output = Result<u32>> + Send;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl TableStore for crate::WasiProvider {
    async fn query(&self, conn_name: &str, query: &str, params: &[DataType]) -> Result<Vec<Row>> {
        let (conn, stmt) = prepare(conn_name, query, params).await?;
        let rows = omnia_wasi_sql::readwrite::query(&conn, &stmt)
            .await
            .map_err(|e| anyhow::anyhow!("query failed: {}", e.trace()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn exec(&self, conn_name: &str, query: &str, params: &[DataType]) -> Result<u32> {
        let (conn, stmt) = prepare(conn_name, query, params).await?;
        omnia_wasi_sql::readwrite::exec(&conn, &stmt)
            .await
            .map_err(|e| anyhow::anyhow!("exec failed: {}", e.trace()))
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
async fn prepare(
    conn_name: &str, query: &str, params: &[DataType],
) -> Result<(omnia_wasi_sql::types::Connection, omnia_wasi_sql::types::Statement)> {
    use omnia_wasi_sql::types::{Connection, Statement};

    let conn = Connection::open(conn_name.to_owned())
        .await
        .map_err(|e| anyhow::anyhow!("failed to open connection: {}", e.trace()))?;
    let params = params.iter().cloned().map(Into::into).collect();
    let stmt = Statement::prepare(query.to_owned(), params)
        .await
        .map_err(|e| anyhow::anyhow!("failed to prepare statement: {}", e.trace()))?;
    Ok((conn, stmt))
}

/// Typed row decoding over any [`TableStore`].
///
/// Blanket-implemented for every [`TableStore`]; never implemented by hand.
pub trait TableStoreExt: TableStore {
    /// Executes a query and decodes each returned row through [`FromRow`].
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or any row cannot be decoded as `T`.
    fn query_as<T: FromRow>(
        &self, conn_name: &str, query: &str, params: &[DataType],
    ) -> impl Future<Output = Result<Vec<T>>> + Send {
        async move {
            let rows = self.query(conn_name, query, params).await?;
            rows.iter().map(T::from_row).collect()
        }
    }
}

impl<P: TableStore + ?Sized> TableStoreExt for P {}
