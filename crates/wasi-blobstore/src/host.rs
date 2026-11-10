//! # WASI Blobstore Service

mod blobstore_impl;
mod container_impl;
mod default_impl;
mod resource;
mod types_impl;

mod generated {
    pub use super::{ContainerProxy, Error, IncomingValue, OutgoingValue, StreamObjectNames};

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

pub use bytes::Bytes;
pub use default_impl::BlobStoreDefault;
pub use omnia_backend::blobstore::{BlobStoreClient, Container, ContainerMetadata, ObjectMetadata};
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
pub use resource::*;
use wasmtime::component::Linker;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

use self::generated::wasi::blobstore::{blobstore, container, types};

/// Typed host error for `wasi:blobstore`, lowered to the WIT `error` string.
#[derive(Debug)]
pub enum Error {
    /// A requested container or object does not exist.
    NotFound(String),
    /// Any other host failure; the message preserves the backend context chain.
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(msg) | Self::Other(msg) => f.write_str(msg),
        }
    }
}

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
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:blobstore`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = BlobStoreClient)]
pub struct WasiBlobStore;

impl<B: Provides<Self>> Host<B> for WasiBlobStore {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        blobstore::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        container::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

omnia_core::host_error!(Error, Other);
