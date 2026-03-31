//! # WASI Blobstore Service

mod blobstore_impl;
mod container_impl;
pub mod default_impl;
mod resource;
mod types_impl;

mod generated {
    #![allow(missing_docs)]

    pub type Error = String;

    pub use super::{ContainerProxy, IncomingValue, OutgoingValue, StreamObjectNames};

    wasmtime::component::bindgen!({
        world: "imports",
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
            "wasi:blobstore/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

use bytes::Bytes;
pub use omnia::FutureResult;
use omnia::{Host, Server, State};
pub use resource::*;
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

pub use self::default_impl::BlobstoreDefault;
pub use self::generated::wasi::blobstore::container::{ContainerMetadata, ObjectMetadata};
pub use self::generated::wasi::blobstore::types::Error;
use self::generated::wasi::blobstore::{blobstore, container, types};

/// Incoming value for a blobstore operation.
pub type IncomingValue = Bytes;
/// Outgoing value for a blobstore operation.
#[derive(Debug, Clone)]
pub struct OutgoingValue {
    pub(crate) pipe: MemoryOutputPipe,
    pub(crate) write_body_taken: bool,
    pub(crate) finished: bool,
}

impl OutgoingValue {
    /// Create a new outgoing value with an in-memory buffer capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            pipe: MemoryOutputPipe::new(capacity),
            write_body_taken: false,
            finished: false,
        }
    }

    pub(crate) const fn take_write_body(&mut self) -> std::result::Result<(), ()> {
        if self.finished || self.write_body_taken {
            return Err(());
        }
        self.write_body_taken = true;
        Ok(())
    }

    pub(crate) const fn finalize(&mut self) -> std::result::Result<(), &'static str> {
        if self.finished {
            return Err("outgoing value already finished");
        }
        if !self.write_body_taken {
            return Err("outgoing value write body was never requested");
        }
        self.finished = true;
        Ok(())
    }
}
/// Stream of object names with position tracking for paginated reads.
pub struct StreamObjectNames {
    /// The full list of object names in this stream.
    pub(crate) names: Vec<String>,
    /// Current read offset into `names`.
    pub(crate) offset: usize,
}

impl StreamObjectNames {
    /// Create a new stream from a complete list of object names.
    #[must_use]
    pub const fn new(names: Vec<String>) -> Self {
        Self { names, offset: 0 }
    }
}

/// Result type for blobstore operations.
pub type Result<T> = anyhow::Result<T, Error>;

/// Host-side service for `wasi:blobstore`.
#[derive(Debug)]
pub struct WasiBlobstore;

impl HasData for WasiBlobstore {
    type Data<'a> = WasiBlobstoreCtxView<'a>;
}

impl<T> Host<T> for WasiBlobstore
where
    T: WasiBlobstoreView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        blobstore::add_to_linker::<_, Self>(linker, T::blobstore)?;
        container::add_to_linker::<_, Self>(linker, T::blobstore)?;
        Ok(types::add_to_linker::<_, Self>(linker, T::blobstore)?)
    }
}

impl<S> Server<S> for WasiBlobstore where S: State {}

/// A trait which provides internal WASI Blobstore state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiBlobstoreView: Send {
    /// Return a [`WasiBlobstoreCtxView`] from mutable reference to self.
    fn blobstore(&mut self) -> WasiBlobstoreCtxView<'_>;
}

/// View into [`WasiBlobstoreCtx`] implementation and [`ResourceTable`].
pub struct WasiBlobstoreCtxView<'a> {
    /// Mutable reference to the WASI Blobstore context.
    pub ctx: &'a mut dyn WasiBlobstoreCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

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

/// Implementation of the `WasiBlobstoreView` trait for the store context.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl omnia_wasi_blobstore::WasiBlobstoreView for $store_ctx {
            fn blobstore(&mut self) -> omnia_wasi_blobstore::WasiBlobstoreCtxView<'_> {
                omnia_wasi_blobstore::WasiBlobstoreCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}

// impl<'a, T> CtxView<'a, T> for WasiBlobstore
// where
//     T: WasiBlobstoreCtx,
// {
//     fn ctx_view(ctx: &'a mut T, table: &'a mut ResourceTable) -> WasiBlobstoreCtxView<'a> {
//         WasiBlobstoreCtxView { ctx, table }
//     }
// }

// #[macro_export]
// macro_rules! omnia_wasi_view {
//     ($store_ctx:ty, $field_name:ident) => {
//         impl View<WasiBlobstore, $store_ctx> for $store_ctx {
//             fn data(&mut self) -> <WasiBlobstore as HasData>::Data<'_> {
//                 WasiBlobstore::ctx_view(&mut self.$field_name, &mut self.table)
//             }
//         }
//     };
// }

#[cfg(test)]
mod tests {
    use super::OutgoingValue;

    #[test]
    fn outgoing_value_write_body_is_one_shot() {
        let mut outgoing = OutgoingValue::new(16);
        assert_eq!(outgoing.take_write_body(), Ok(()));
        assert_eq!(outgoing.take_write_body(), Err(()));
    }

    #[test]
    fn outgoing_value_finish_requires_write_body() {
        let mut outgoing = OutgoingValue::new(16);
        assert_eq!(outgoing.finalize(), Err("outgoing value write body was never requested"));
    }

    #[test]
    fn outgoing_value_finish_is_single_use() {
        let mut outgoing = OutgoingValue::new(16);
        assert_eq!(outgoing.take_write_body(), Ok(()));
        assert_eq!(outgoing.finalize(), Ok(()));
        assert_eq!(outgoing.finalize(), Err("outgoing value already finished"));
    }
}
