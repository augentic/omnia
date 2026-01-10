//! # WASI Blobstore Service

mod blobstore_impl;
mod container_impl;
pub mod default_impl;
mod resource;
mod types_impl;

mod generated {
    #![allow(clippy::trait_duplication_in_bounds)]

    pub use super::{ContainerProxy, IncomingValue, OutgoingValue, StreamObjectNames};

    wasmtime::component::bindgen!({
        world: "blobstore",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:io": wasmtime_wasi::p2::bindings::io,
            "wasi:blobstore/types.incoming-value": IncomingValue,
            "wasi:blobstore/types.outgoing-value": OutgoingValue,
            "wasi:blobstore/container.container": ContainerProxy,
            "wasi:blobstore/container.stream-object-names": StreamObjectNames,
        },
        trappable_error_type: {
            "wasi:blobstore/types.error" => anyhow::Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
pub use resource::*;
pub use yetti::FutureResult;
use yetti::{Host, Server, State};
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

pub use self::default_impl::BlobstoreDefault;
pub use self::generated::wasi::blobstore::container::{ContainerMetadata, ObjectMetadata};
use self::generated::wasi::blobstore::{blobstore, container, types};

pub type IncomingValue = Bytes;
pub type OutgoingValue = MemoryOutputPipe;
pub type StreamObjectNames = Vec<String>;

#[derive(Debug)]
pub struct WasiBlobstore;

impl HasData for WasiBlobstore {
    type Data<'a> = WasiBlobstoreCtxView<'a>;
}

impl<T> Host<T> for WasiBlobstore
where
    T: WasiBlobstoreView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()> {
        blobstore::add_to_linker::<_, Self>(linker, T::blobstore)?;
        container::add_to_linker::<_, Self>(linker, T::blobstore)?;
        types::add_to_linker::<_, Self>(linker, T::blobstore)
    }
}

impl<S> Server<S> for WasiBlobstore where S: State {}

/// A trait which provides internal WASI Blobstore context.
///
/// This is implemented by the resource-specific provider of Blobstore
/// functionality. For example, an in-memory store, or a Redis-backed store.
pub trait WasiBlobstoreCtx: Debug + Send + Sync + 'static {
    /// Open a container.
    fn create_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;

    /// Get a container.
    fn get_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;

    /// Delete a container.
    fn delete_container(&self, name: String) -> FutureResult<()>;

    /// Check if a container exists.
    fn container_exists(&self, name: String) -> FutureResult<bool>;
}

/// View into [`WasiBlobstoreCtx`] implementation and [`ResourceTable`].
pub struct WasiBlobstoreCtxView<'a> {
    /// Mutable reference to the WASI Blobstore context.
    pub ctx: &'a mut dyn WasiBlobstoreCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI Blobstore state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiBlobstoreView: Send {
    /// Return a [`WasiBlobstoreCtxView`] from mutable reference to self.
    fn blobstore(&mut self) -> WasiBlobstoreCtxView<'_>;
}

#[macro_export]
macro_rules! wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl yetti_wasi_blobstore::WasiBlobstoreView for $store_ctx {
            fn blobstore(&mut self) -> yetti_wasi_blobstore::WasiBlobstoreCtxView<'_> {
                yetti_wasi_blobstore::WasiBlobstoreCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
