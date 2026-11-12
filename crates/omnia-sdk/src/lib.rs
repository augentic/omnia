#![doc = include_str!("../README.md")]

pub mod api;
pub mod blobstore;
mod capabilities;
pub mod capability;
pub mod contract;
pub mod docstore;
mod error;
pub mod identity;
pub mod keyvalue;
#[cfg(feature = "http")]
pub mod mcp;
pub mod messaging;
pub mod sql;
pub mod vault;

#[cfg(feature = "http")]
#[doc(hidden)]
pub use axum;
pub use omnia_guest_macros::{Provider, Settings, capability};

pub use crate::capabilities::*;
pub use crate::error::*;
#[doc(hidden)]
pub mod http {
    pub use ::http::*;
}
#[doc(hidden)]
pub use {anyhow, bytes, http_body, tracing};
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
pub use {
    omnia_wasi_blobstore, omnia_wasi_http, omnia_wasi_identity, omnia_wasi_keyvalue,
    omnia_wasi_messaging, omnia_wasi_otel, omnia_wasi_vault, omnia_wasi_vcs, wasip3, wit_bindgen,
};
