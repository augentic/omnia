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
        world: "sql",
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

pub use qwasr::FutureResult;
use qwasr::{Host, Server, State};
use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::ResourceTable;

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
        types::add_to_linker::<_, Self>(linker, T::sql)
    }
}

impl<S> Server<S> for WasiSql where S: State {}

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

/// Implementation of the `WasiSqlView` trait for the store context.
#[macro_export]
macro_rules! qwasr_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl qwasr_wasi_sql::WasiSqlView for $store_ctx {
            fn sql(&mut self) -> qwasr_wasi_sql::WasiSqlCtxView<'_> {
                qwasr_wasi_sql::WasiSqlCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
