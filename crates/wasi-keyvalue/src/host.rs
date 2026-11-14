//! # WASI Key-Value Service

mod atomics_impl;
mod batch_impl;
mod default_impl;
mod omnia_impl;
mod resource;
mod store_impl;

mod generated {
    pub use self::omnia::keyvalue::store::Error;
    pub use self::wasi::keyvalue::atomics::CasError;
    pub use self::wasi::keyvalue::store::Error as WasiError;
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
            "omnia:keyvalue/store.error" => Error,
            "wasi:keyvalue/store.error" => WasiError,
            "wasi:keyvalue/atomics.cas-error" => CasError,
        },

    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::{Linker, ResourceTableError};

pub use self::default_impl::{ConnectOptions, KeyValueDefault};
pub use self::generated::Error;
use self::generated::WasiError;
use self::generated::omnia::keyvalue::store as omnia_store;
use self::generated::wasi::keyvalue::{atomics, batch, store};
pub use self::resource::*;

type Result<T> = std::result::Result<T, Error>;
type WasiResult<T> = std::result::Result<T, WasiError>;

/// Host-side service for `omnia:keyvalue` and the `wasi:keyvalue` draft.
#[derive(Debug, HostCtx)]
#[host_ctx(client = KeyValueClient)]
pub struct WasiKeyValue;

impl<B: Provides<Self>> Host<B> for WasiKeyValue {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        atomics::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(batch::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
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

// a backend picks the kind by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}

// the draft names only these two kinds; the rest collapse into `other`
impl From<anyhow::Error> for WasiError {
    fn from(err: anyhow::Error) -> Self {
        match err.downcast_ref::<Error>() {
            Some(Error::NotFound(_)) => Self::NoSuchStore,
            Some(Error::Denied(_)) => Self::AccessDenied,
            _ => Self::Other(format!("{err:#}")),
        }
    }
}

impl From<ResourceTableError> for WasiError {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}
