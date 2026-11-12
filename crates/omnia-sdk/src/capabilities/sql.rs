//! Relational database capability.

use std::future::Future;

use crate::capability::Result;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::sql::{DEFAULT, Handle};
use crate::sql::{FromRow, Row, Statement, Value};

/// Run parameterized SQL against the databases a deployment declares.
///
/// A handle runs statements on one database. A provider's own handle uses
/// [`sql::DEFAULT`](crate::sql::DEFAULT), and [`named`](Self::named) selects
/// another. A database the deployment does not declare fails on first use.
/// Typed row decoding is [`SqlDbExt`].
#[omnia_guest_macros::capability]
pub trait SqlDb: Send + Sync {
    /// The handle [`named`](Self::named) returns.
    type Named: SqlDb<Named = Self::Named>;

    /// Run `sql` with `params` bound and return the rows it selects.
    fn query(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<Vec<Row>>> + Send;

    /// Run `sql` with `params` bound and return the number of rows it changed.
    fn execute(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<u64>> + Send;

    /// Run `statements` in one transaction and return each one's changed-row count.
    ///
    /// A failing statement rolls back the whole batch and returns its error.
    /// A provider without transactions fails with
    /// [`ErrorKind::Unsupported`](crate::capability::ErrorKind::Unsupported).
    fn batch(&self, statements: &[Statement]) -> impl Future<Output = Result<Vec<u64>>> + Send;

    /// A handle to the database `name` on the same provider.
    fn named(&self, name: &str) -> Self::Named;
}

/// Typed row decoding over any [`SqlDb`].
///
/// Blanket-implemented for every [`SqlDb`]; never implemented by hand.
pub trait SqlDbExt: SqlDb {
    /// Run `sql` with `params` bound and decode each row it selects as `T`.
    ///
    /// A row that does not decode fails with
    /// [`ErrorKind::Invalid`](crate::capability::ErrorKind::Invalid).
    fn query_as<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = Result<Vec<T>>> + Send {
        async move { self.query(sql, params).await?.iter().map(T::from_row).collect() }
    }
}

impl<P: SqlDb + ?Sized> SqlDbExt for P {}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl SqlDb for crate::WasiProvider {
    type Named = Handle;

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        Handle::new(DEFAULT).query(sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Handle::new(DEFAULT).execute(sql, params).await
    }

    async fn batch(&self, statements: &[Statement]) -> Result<Vec<u64>> {
        Handle::new(DEFAULT).batch(statements).await
    }

    fn named(&self, name: &str) -> Handle {
        Handle::new(name)
    }
}
