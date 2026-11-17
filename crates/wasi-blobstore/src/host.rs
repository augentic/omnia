//! # WASI Blobstore Service

mod admin_impl;
mod container_impl;
mod default_impl;
mod presign_impl;
mod resource;
mod store_impl;
mod transfer;
mod uploads_impl;

mod generated {
    #![allow(missing_docs, clippy::empty_enums, reason = "wasmtime bindgen output")]

    pub use self::omnia::blobstore::types::Error;
    pub use super::container_impl::ContainerHandle;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:blobstore/container.container": ContainerHandle,
        },
        trappable_error_type: {
            "omnia:blobstore/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use bytes::Bytes;
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
pub use resource::*;
use wasmtime::component::{Linker, ResourceTableError};

pub use self::default_impl::BlobStoreDefault;
pub use self::generated::Error;
use self::generated::omnia::blobstore::types::Host as TypesHost;
use self::generated::omnia::blobstore::{admin, container, presign, store, types, uploads};

impl Error {
    /// The [`NotFound`](Self::NotFound) error for the container `name`, which does not exist.
    #[must_use]
    pub fn missing_container(name: &str) -> Self {
        Self::NotFound(format!("container not found: {name}"))
    }
}

// a backend picks the kind by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
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

/// Result type for blobstore operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:blobstore`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = BlobStoreClient)]
pub struct WasiBlobStore;

impl<B: Provides<Self>> Host<B> for WasiBlobStore {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        container::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        admin::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        uploads::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(presign::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for blobstore backends.
///
/// This is implemented by the resource-specific provider of Blobstore
/// functionality. For example, an in-memory store, or a Redis-backed store.
///
/// A backend picks a failure's kind by returning an [`Error`] inside its
/// [`anyhow::Error`], and any other failure is [`Error::Other`].
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

impl TypesHost for HostView<'_, dyn BlobStoreClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
