//! # Host implementation for WASI SQL Service
//!
//! This module implements the host side of `omnia:sql` and the `wasi:sql`
//! draft.

#[cfg(feature = "sqlite")]
mod default_impl;
mod omnia_impl;
mod readwrite_impl;
mod resource;
mod types_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use self::omnia::sql::database::Error;
    pub use super::resource::{ConnectionProxy, Prepared, Trace};

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:sql/types.connection": ConnectionProxy,
            "wasi:sql/types.statement": Prepared,
            "wasi:sql/types.error": Trace,
        },
        trappable_error_type: {
            "omnia:sql/database.error" => Error,
            "wasi:sql/types.error" => Trace,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::Linker;

#[cfg(feature = "sqlite")]
pub use self::default_impl::{ConnectOptions, SqlDefault};
pub use self::generated::Error;
use self::generated::omnia::sql::database;
pub use self::generated::omnia::sql::database::{Field, Row, Statement, Value};
use self::generated::wasi::sql::{readwrite, types};
pub use self::resource::{Connection, ConnectionProxy};

type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:sql` and the `wasi:sql` draft.
#[derive(Debug, HostCtx)]
#[host_ctx(client = SqlClient)]
pub struct WasiSql;

impl<B: Provides<Self>> Host<B> for WasiSql {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        database::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        readwrite::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
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
