//! # Host implementation for WASI SQL Service
//!
//! This module implements the host-side logic for the WASI SQL service.

#[cfg(feature = "sqlite")]
mod default_impl;
mod readwrite_impl;
mod resource;
mod types_impl;

mod generated {
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

#[cfg(feature = "sqlite")]
pub use default_impl::{ConnectOptions, SqlDefault};
pub use omnia_backend::sql::{Connection, DataType, Field, Row, SqlClient};
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::Linker;

use self::generated::wasi::sql::{readwrite, types};
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

/// Host-side service for `wasi:sql`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = SqlClient)]
pub struct WasiSql;

impl<B: Provides<Self>> Host<B> for WasiSql {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        readwrite::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

omnia_core::host_error!(Error, Other);
