//! Host-side `wasi:jsondb` implementation.

pub mod default_impl;
mod resource;
mod store_impl;
mod types_impl;

/// Errors surfaced through the WIT `error` type.
#[derive(Debug, Clone)]
pub enum JsonDbError {
    /// Store or collection not found.
    NoSuchStore,
    /// Operation not permitted.
    AccessDenied,
    /// Other failure with message.
    Other(String),
}

impl std::fmt::Display for JsonDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchStore => write!(f, "no such store"),
            Self::AccessDenied => write!(f, "access denied"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for JsonDbError {}

mod generated {
    #![allow(missing_docs)]

    pub use super::{FilterProxy, JsonDbError};

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:jsondb/types.filter": FilterProxy,
        },
        trappable_error_type: {
            "wasi:jsondb/types.error" => JsonDbError,
        },
    });
}

use std::fmt::Debug;

pub use omnia::FutureResult;
use omnia::{Host, Server, State};
use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::ResourceTable;

use self::generated::wasi::jsondb::{store, types};
pub use crate::host::default_impl::JsonDbDefault;
pub use crate::host::generated::wasi::jsondb::types::{
    ComparisonOp, Document, QueryResult, ScalarValue, SortField,
};
pub use crate::host::resource::{FilterProxy, FilterTree};

/// Host service for `wasi:jsondb`.
#[derive(Debug)]
pub struct WasiJsonDb;

impl HasData for WasiJsonDb {
    type Data<'a> = WasiJsonDbCtxView<'a>;
}

impl<T> Host<T> for WasiJsonDb
where
    T: WasiJsonDbView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, T::jsondb)?;
        Ok(store::add_to_linker::<_, Self>(linker, T::jsondb)?)
    }
}

impl<S> Server<S> for WasiJsonDb where S: State {}

/// Mutable view into JSON DB state for the linker.
pub trait WasiJsonDbView: Send {
    /// JSON DB context and resource table.
    fn jsondb(&mut self) -> WasiJsonDbCtxView<'_>;
}

/// View combining [`WasiJsonDbCtx`] and the component [`ResourceTable`].
pub struct WasiJsonDbCtxView<'a> {
    /// Backend context.
    pub ctx: &'a mut dyn WasiJsonDbCtx,

    /// Resource table for `filter` handles.
    pub table: &'a mut ResourceTable,
}

/// Backend operations for JSON document storage.
///
/// All methods use [`anyhow::Error`] for failures; the host maps those to [`JsonDbError`].
pub trait WasiJsonDbCtx: Debug + Send + Sync + 'static {
    /// Point read by primary id.
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>>;

    /// Insert if absent.
    fn insert(&self, collection: String, doc: Document) -> FutureResult<()>;

    /// Upsert by id.
    fn put(&self, collection: String, doc: Document) -> FutureResult<()>;

    /// Delete by id; `Ok(true)` if a document was removed.
    fn delete(&self, collection: String, id: String) -> FutureResult<bool>;

    /// Filtered listing with sort and pagination.
    fn query(
        &self, collection: String, filter: Option<FilterTree>, options: QueryOpts,
    ) -> FutureResult<QueryResult>;
}

/// Host-side query options (WIT `query-options` without the filter resource).
#[derive(Debug, Clone, Default)]
pub struct QueryOpts {
    /// Sort fields.
    pub order_by: Vec<generated::wasi::jsondb::types::SortField>,
    /// Max documents.
    pub limit: Option<u32>,
    /// Skip count (when no continuation token).
    pub offset: Option<u32>,
    /// Opaque continuation.
    pub continuation: Option<String>,
}

/// Implementation of [`WasiJsonDbView`] for generated store contexts.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl $crate::WasiJsonDbView for $store_ctx {
            fn jsondb(&mut self) -> $crate::WasiJsonDbCtxView<'_> {
                $crate::WasiJsonDbCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
