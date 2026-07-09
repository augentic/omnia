#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub mod api;
mod capabilities;
mod error;
pub mod mcp;
pub mod orm;

/// Document store types and helpers (from `omnia-wasi-docstore`).
pub mod document_store {
    pub use omnia_wasi_docstore::document_store::*;
}

pub use omnia_guest_macros::*;
#[doc(hidden)]
pub use {anyhow, axum, bytes, http, http_body, tracing};
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use {
    omnia_wasi_blobstore, omnia_wasi_http, omnia_wasi_identity, omnia_wasi_keyvalue,
    omnia_wasi_messaging, omnia_wasi_otel, wasip3, wit_bindgen,
};

pub use crate::api::*;
pub use crate::capabilities::*;
pub use crate::error::*;
