#![doc = include_str!("../README.md")]

//! # WASI Blobstore Service
//!
//! This module implements a runtime service for `omnia:blobstore`.

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

    pub use omnia_wasi_types::Error;

    pub use super::container_impl::ContainerHandle;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:blobstore/container.container": ContainerHandle,
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use bytes::Bytes;
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
pub use resource::*;
use wasmtime::component::Linker;

pub use self::default_impl::BlobStoreDefault;
pub use self::generated::Error;
use self::generated::omnia::blobstore::types::Host as TypesHost;
use self::generated::omnia::blobstore::{admin, container, presign, store, types, uploads};

/// Result type for blobstore operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:blobstore`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = BlobStoreClient)]
pub struct WasiBlobStore;

impl<B: Provides<Self>> Host<B> for WasiBlobStore {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        container::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        admin::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        uploads::add_to_linker::<_, Self>(
            linker,
            uploads::LinkOptions::default().uploads(true),
            StoreCtx::view::<Self>,
        )?;
        Ok(presign::add_to_linker::<_, Self>(
            linker,
            presign::LinkOptions::default().presign(true),
            StoreCtx::view::<Self>,
        )?)
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
    /// A container that does not exist fails with [`missing_container`],
    /// here or at the first call on the container returned.
    fn get_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;

    /// Delete the container `name` and every object in it, succeeding when it is already absent.
    fn delete_container(&self, name: String) -> FutureResult<()>;

    /// Check if a container exists.
    fn container_exists(&self, name: String) -> FutureResult<bool>;
}

impl TypesHost for HostView<'_, dyn BlobStoreClient> {}

/// The [`Error::NotFound`] for the container `name`, which does not exist.
#[must_use]
pub fn missing_container(name: &str) -> Error {
    Error::NotFound(format!("container not found: {name}"))
}
