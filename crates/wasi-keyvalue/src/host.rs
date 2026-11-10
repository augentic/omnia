//! # WASI Key-Value Service

mod atomics_impl;
mod batch_impl;
mod default_impl;
mod resource;
mod store_impl;
mod ttl_impl;

mod generated {
    pub use self::wasi::keyvalue::atomics::CasError;
    pub use self::wasi::keyvalue::store::Error;
    pub use super::{BucketProxy, Cas};

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:keyvalue/store.bucket": BucketProxy,
            "wasi:keyvalue/atomics.cas": Cas,
        },
        trappable_error_type: {
            "wasi:keyvalue/store.error" => Error,
            "wasi:keyvalue/atomics.cas-error" => CasError,
        },

    });
}

use std::fmt::Debug;

pub use default_impl::KeyValueDefault;
pub use omnia_backend::keyvalue::{Bucket, KeyValueClient};
pub use omnia_core::FutureResult;
use omnia_core::{Backends, Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::Linker;

use self::generated::omnia::keyvalue::ttl;
use self::generated::wasi::keyvalue::store::Error;
use self::generated::wasi::keyvalue::{atomics, batch, store};
pub use self::resource::*;

/// Result type for key-value operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:keyvalue`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = KeyValueClient)]
pub struct WasiKeyValue;

impl<B: Backends + Provides<Self>> Host<B> for WasiKeyValue {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        atomics::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        batch::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(ttl::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

omnia_core::host_error!(Error, Other);
