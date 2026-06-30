//! # Host implementation for WASI SQL Service
//!
//! This module implements the host-side logic for the WASI SQL service.

pub mod default_impl;
mod readwrite_impl;
mod resource;
mod types_impl;

mod generated {
    #![allow(missing_docs)]

    pub use anyhow::Error;

    pub use super::{ConnectionProxy, Statement};

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
use wasmtime::component::{HasData, Linker, ResourceTable};

use self::generated::wasi::sql::{readwrite, types};
pub use crate::host::default_impl::SqlDefault;
pub use crate::host::generated::wasi::sql::types::{DataType, Field, Row};
pub use crate::host::resource::*;

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

/// A trait which provides internal WASI SQL state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiSqlView: Send {
    /// Return a [`WasiSqlCtxView`] from mutable reference to self.
    fn sql(&mut self) -> WasiSqlCtxView<'_>;
}

/// View into [`WasiSqlCtx`] implementation and [`ResourceTable`].
pub struct WasiSqlCtxView<'a> {
    /// Mutable reference to the WASI SQL context.
    pub ctx: &'a mut dyn WasiSqlCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI SQL context.
///
/// This is implemented by the resource-specific provider of SQL
/// functionality. For example, `PostgreSQL`, `MySQL`, `SQLite`, etc.
pub trait WasiSqlCtx: Debug + Send + Sync + 'static {
    /// Open a connection to the database.
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>>;
}

/// A backend bundle that can yield the `wasi:sql` backend for a store.
///
/// The blanket [`WasiSqlView`] impl below turns this accessor into the
/// linker-facing view on `omnia::StoreCtx<B>`; the `runtime!` macro generates
/// the bundle-side impl via `omnia_wasi_view!`.
pub trait HasSql: Send {
    /// Borrow the `wasi:sql` backend context.
    fn sql_ctx(&mut self) -> &mut dyn WasiSqlCtx;
}

impl<B: HasSql + Send + 'static> WasiSqlView for omnia::StoreCtx<B> {
    fn sql(&mut self) -> WasiSqlCtxView<'_> {
        WasiSqlCtxView {
            ctx: self.backends.sql_ctx(),
            table: &mut self.base.table,
        }
    }
}

/// Generates the bundle's [`HasSql`] impl for a `runtime!` deployment.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl $crate::HasSql for $bundle {
            fn sql_ctx(&mut self) -> &mut dyn $crate::WasiSqlCtx {
                &mut self.$field_name
            }
        }
    };
}
