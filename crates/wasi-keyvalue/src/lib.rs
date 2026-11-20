#![doc = include_str!("../README.md")]

//! # WASI Key-Value
//!
//! This module implements a runtime service for `omnia:keyvalue`.

mod bucket_impl;
mod default_impl;
mod keys_impl;
mod resource;
mod store_impl;

mod generated {
    pub use omnia_wasi_types::Error;

    pub use super::bucket_impl::BucketHandle;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:keyvalue/bucket.bucket": BucketHandle,
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::Linker;

pub use self::default_impl::{ConnectOptions, KeyValueDefault};
pub use self::generated::Error;
use self::generated::omnia::keyvalue::types::Host as TypesHost;
use self::generated::omnia::keyvalue::{bucket, keys, store, types};
pub use self::resource::*;

type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:keyvalue`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = KeyValueClient)]
pub struct WasiKeyValue;

impl<B: Provides<Self>> Host<B> for WasiKeyValue {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        bucket::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(keys::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for key-value backends.
///
/// This is implemented by the resource-specific provider of Key-Value
/// functionality. For example, an in-memory store, or a Redis-backed store.
pub trait KeyValueClient: Debug + Send + Sync + 'static {
    /// Open a bucket the deployment declares.
    ///
    /// An identifier the deployment does not declare fails with
    /// [`Error::NotFound`].
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>>;
}

impl TypesHost for HostView<'_, dyn KeyValueClient> {}
