#![doc = include_str!("../README.md")]

pub mod api;
mod capabilities;
mod error;
#[cfg(feature = "http")]
pub mod mcp;

/// Document store types and helpers (deprecated alias for [`docstore`]).
#[cfg(feature = "docstore")]
#[deprecated(note = "use `omnia_sdk::docstore`")]
pub mod document_store {
    pub use omnia_backend::docstore::*;
}

#[cfg(feature = "http")]
#[doc(hidden)]
pub use axum;
#[cfg(feature = "docstore")]
pub use omnia_backend::docstore;
#[cfg(feature = "sql")]
pub use omnia_backend::sql;
pub use omnia_backend::{blobstore, identity, keyvalue, messaging, vault, websocket};
pub use omnia_guest_macros::Provider;
#[doc(hidden)]
pub use {anyhow, bytes, http, http_body, tracing};
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use {
    omnia_wasi_blobstore, omnia_wasi_http, omnia_wasi_identity, omnia_wasi_keyvalue,
    omnia_wasi_messaging, omnia_wasi_otel, omnia_wasi_vcs, wasip3, wit_bindgen,
};

#[cfg(feature = "http")]
pub use crate::api::http::{HttpError, HttpResult};
pub use crate::capabilities::*;
pub use crate::error::*;
