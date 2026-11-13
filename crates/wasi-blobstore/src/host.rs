//! # WASI Blobstore Service

mod blobstore_impl;
mod container_impl;
mod default_impl;
mod resource;
mod types_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

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
use std::sync::Arc;

pub use bytes::Bytes;
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
pub use resource::*;
use wasmtime::component::{Linker, ResourceTableError};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

pub use self::default_impl::BlobStoreDefault;
pub use self::generated::wasi::blobstore::container::{ContainerMetadata, ObjectMetadata};
use self::generated::wasi::blobstore::{blobstore, container, types};

/// Typed host error for `wasi:blobstore`, lowered to the WIT `error` string.
///
/// A backend picks the kind by returning an [`Error`] inside its
/// [`anyhow::Error`], and any other failure is [`Other`](Self::Other). The
/// message of either keeps the backend's full context chain.
#[derive(Debug)]
pub enum Error {
    /// A requested container or object does not exist.
    NotFound(String),
    /// Any other host failure.
    Other(String),
}

impl Error {
    /// The [`NotFound`](Self::NotFound) error for the container `name`, which does not exist.
    #[must_use]
    pub fn missing_container(name: &str) -> Self {
        Self::NotFound(format!("container not found: {name}"))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(msg) | Self::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        let message = format!("{err:#}");
        match err.downcast_ref::<Self>() {
            Some(Self::NotFound(_)) => Self::NotFound(message),
            Some(Self::Other(_)) | None => Self::Other(message),
        }
    }
}

impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Self::Other(format!("{err:#}"))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
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

/// Service contract for `wasi:blobstore` backends.
///
/// This is implemented by the resource-specific provider of Blobstore
/// functionality. For example, an in-memory store, or a Redis-backed store.
pub trait BlobStoreClient: Debug + Send + Sync + 'static {
    /// Create the container `name`, or open it when it already exists.
    fn create_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;

    /// Open the container `name`.
    ///
    /// A container that does not exist fails with
    /// [`Error::missing_container`], here or at the first call on the
    /// container returned.
    fn get_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;

    /// Delete the container `name` and every object in it, succeeding when it is already absent.
    fn delete_container(&self, name: String) -> FutureResult<()>;

    /// Check if a container exists.
    fn container_exists(&self, name: String) -> FutureResult<bool>;
}
