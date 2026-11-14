//! [`WasiProvider`] and its capability impls over the WASI host.
//!
//! On `wasm32`, `WasiProvider` serves each capability through the matching
//! `omnia-wasi-*` guest bindings. A capability that addresses named bindings
//! returns a `Handle` from its `named`:
//!
//! - `identity::Handle`
//! - `keyvalue::Handle`
//! - `sql::Handle`
//! - `vault::Handle`

cfg_select! {
    all(target_arch = "wasm32", target_os = "wasi") => {
        mod blobstore;
        mod config;
        mod docstore;
        mod http;
        pub mod identity;
        pub mod keyvalue;
        mod lend;
        pub(crate) mod messaging;
        mod model;
        mod plugins;
        pub mod sql;
        pub mod vault;
        mod vcs;
        mod websockets;
    }
    _ => {}
}

/// Zero-sized WASI provider implementing every capability trait on `wasm32`.
///
/// Delegates to the `omnia-wasi-*` guest bindings on `wasm32-wasip2`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WasiProvider;
