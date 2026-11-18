//! # WASI SQL Service

mod database_impl;
#[cfg(feature = "sqlite")]
mod default_impl;
mod resource;
mod store_impl;
mod transactions_impl;

mod generated {
    pub use self::omnia::sql::types::Error;
    pub use super::database_impl::DatabaseHandle;
    pub use super::transactions_impl::TransactionHandle;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:sql/database.database": DatabaseHandle,
            "omnia:sql/transactions.transaction": TransactionHandle,
        },
        trappable_error_type: {
            "omnia:sql/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::{Linker, ResourceTableError};

#[cfg(feature = "sqlite")]
pub use self::default_impl::{ConnectOptions, SqlDefault};
pub use self::generated::Error;
use self::generated::omnia::sql::types::Host as TypesHost;
use self::generated::omnia::sql::{database, store, transactions, types};
pub use self::resource::*;

type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:sql`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = SqlClient)]
pub struct WasiSql;

impl<B: Provides<Self>> Host<B> for WasiSql {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        database::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(transactions::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for SQL backends.
///
/// This is implemented by the resource-specific provider of SQL
/// functionality. For example, `PostgreSQL`, `MySQL`, `SQLite`, etc.
pub trait SqlClient: Debug + Send + Sync + 'static {
    /// Open a connection to the database `name`.
    ///
    /// A name the deployment does not declare fails with
    /// [`Error::NotFound`].
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>>;
}

// a backend picks the kind by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}

impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Self::Other(format!("{err:#}"))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

impl TypesHost for HostView<'_, dyn SqlClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
