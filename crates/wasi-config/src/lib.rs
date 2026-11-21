#![doc = include_str!("../README.md")]

//! # WASI Config Service
//!
//! This module implements a runtime service for `omnia:config`.

mod default_impl;
mod entries_impl;
mod store_impl;

mod generated {
    pub use omnia_wasi_types::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

use futures::{FutureExt as _, future};
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::Linker;

pub use self::default_impl::{ConfigDefault, ConnectOptions};
pub use self::generated::Error;
use self::generated::omnia::config::{entries, store, types};

/// Result type for configuration operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:config`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = ConfigClient)]
pub struct WasiConfig;

impl<B: Provides<Self>> Host<B> for WasiConfig {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(entries::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:config` backends.
///
/// A backend implements [`get`](Self::get), and overrides
/// [`entries`](Self::entries) when it can list what it holds.
///
/// A backend reads every portable key, and may fail any other with
/// [`Error::Invalid`]. A portable key:
///
/// - is 1 to 253 bytes long;
/// - holds only uppercase ASCII letters, digits, and `_`;
/// - does not start with a digit.
///
/// A backend picks the kind of a failure by returning an [`Error`], and any
/// other error reaches the guest as [`Error::Other`].
pub trait ConfigClient: Debug + Send + Sync + 'static {
    /// The value set for `key`, or `None` when it is unset.
    fn get(&self, key: String) -> FutureResult<Option<String>>;

    /// Every entry the guest may read, in any order.
    ///
    /// A backend that does not override it fails with
    /// [`Error::Unsupported`].
    fn entries(&self) -> FutureResult<Vec<(String, String)>> {
        let error = Error::Unsupported("this backend cannot list its entries".to_owned());
        future::ready(Err(error.into())).boxed()
    }
}

impl types::Host for HostView<'_, dyn ConfigClient> {}
