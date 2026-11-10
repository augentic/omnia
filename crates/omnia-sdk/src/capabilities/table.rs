//! Relational (SQL) table capability over `wasi:sql`.

use std::future::Future;

use anyhow::Result;

use crate::sql::{DataType, FromRow, Row};

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

// conversions between the target-independent records and the bindings
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
mod wire {
    use omnia_wasi_sql::types;

    use crate::sql::{DataType, Field, Row};

    impl From<DataType> for types::DataType {
        fn from(value: DataType) -> Self {
            match value {
                DataType::Int32(v) => Self::Int32(v),
                DataType::Int64(v) => Self::Int64(v),
                DataType::Uint32(v) => Self::Uint32(v),
                DataType::Uint64(v) => Self::Uint64(v),
                DataType::Float(v) => Self::Float(v),
                DataType::Double(v) => Self::Double(v),
                DataType::Str(v) => Self::Str(v),
                DataType::Boolean(v) => Self::Boolean(v),
                DataType::Date(v) => Self::Date(v),
                DataType::Time(v) => Self::Time(v),
                DataType::Timestamp(v) => Self::Timestamp(v),
                DataType::Binary(v) => Self::Binary(v),
            }
        }
    }

    impl From<types::DataType> for DataType {
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
                types::DataType::Binary(v) => Self::Binary(v),
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
                index: row.index,
                fields: row.fields.into_iter().map(Into::into).collect(),
            }
        }
    }
}
