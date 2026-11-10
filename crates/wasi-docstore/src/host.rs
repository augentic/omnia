//! # WASI `DocStore` Service

mod default_impl;
mod resource;
mod store_impl;
mod types_impl;

mod generated {
    #![expect(missing_docs, reason = "wasmtime bindgen output")]

    pub use self::omnia::docstore::types::Error;
    pub use super::FilterProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:docstore/types.filter": FilterProxy,
        },
        trappable_error_type: {
            "omnia:docstore/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use default_impl::DocStoreDefault;
pub use omnia_backend::docstore::{
    ComparisonOp, DocStoreClient, Document, Filter, QueryOptions, QueryResult, ScalarValue,
    SortField,
};
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::Linker;

pub use self::generated::omnia::docstore::types::Error;
use self::generated::omnia::docstore::{store, types};
pub use self::resource::*;

/// Result type for docstore operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:docstore`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = DocStoreClient)]
pub struct WasiDocStore;

impl<B: Provides<Self>> Host<B> for WasiDocStore {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

omnia_core::host_error!(Error, Other);
