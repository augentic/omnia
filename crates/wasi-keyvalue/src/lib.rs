#![doc = include_str!("../README.md")]

//! # WASI Key-Value
//!
//! This module implements a runtime service for `wasi:keyvalue`
//! (<https://github.com/WebAssembly/wasi-keyvalue>).

cfg_select! {
    target_arch = "wasm32" => {
        mod guest;
        pub use guest::*;
    }
    _ => {
        mod client;
        #[cfg(feature = "host")]
        mod host;

        pub use omnia_backend::FutureResult;

        pub use self::client::{Bucket, KeyValueClient, add_i64};
        #[cfg(feature = "host")]
        pub use self::host::*;

        /// Host-side service for `wasi:keyvalue`.
        #[derive(Debug)]
        #[cfg_attr(feature = "host", derive(omnia_core::HostCtx), host_ctx(client = KeyValueClient))]
        pub struct WasiKeyValue;

        omnia_backend::capability!(WasiKeyValue, "wasi:keyvalue", KeyValueClient);
    }
}

/// The bucket `StateStore` opens.
///
/// A value written without a TTL stays until deleted, on every backend.
pub const BUCKET: &str = "state";
