//! # Host implementation for WASI SQL Service
//!
//! This module implements the host-side logic for the WASI SQL service.

mod default_impl;
mod readwrite_impl;
mod resource;
mod types_impl;

mod generated {
    #![allow(missing_docs)]

    pub use super::{ConnectionProxy, Error, Statement};

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:sql/types.connection": ConnectionProxy,
            "wasi:sql/types.statement": Statement,
            "wasi:sql/types.error": Error,
        },
        trappable_error_type: {
            "wasi:sql/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, Server};
use wasmtime::component::{HasData, Linker};

use self::generated::wasi::sql::{readwrite, types};
pub use crate::host::default_impl::SqlDefault;
pub use crate::host::generated::wasi::sql::types::{DataType, Field, Row};
pub use crate::host::resource::*;

/// Host error backing the `wasi:sql` error resource.
#[derive(Debug)]
pub enum Error {
    /// Untyped host failure; the message preserves the backend context chain.
    Other(String),
}

impl Error {
    /// The failure trace exposed to guests via the WIT `error` resource.
    #[must_use]
    pub fn trace(&self) -> &str {
        let Self::Other(msg) = self;
        msg
    }
}

omnia::host_error!(Error, Other);

/// Host-side service for `wasi:sql`.
#[derive(Debug)]
pub struct WasiSql;

impl HasData for WasiSql {
    type Data<'a> = WasiSqlCtxView<'a>;
}

impl<T> Host<T> for WasiSql
where
    T: WasiSqlView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        readwrite::add_to_linker::<_, Self>(linker, T::sql)?;
        Ok(types::add_to_linker::<_, Self>(linker, T::sql)?)
    }
}

impl<B> Server<B> for WasiSql {}

/// A trait which provides internal WASI SQL context.
///
/// This is implemented by the resource-specific provider of SQL
/// functionality. For example, `PostgreSQL`, `MySQL`, `SQLite`, etc.
pub trait WasiSqlCtx: Debug + Send + Sync + 'static {
    /// Open a connection to the database.
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>>;
}

omnia::wasi_view!(Sql);
