//! # Realtime Core
//!
//! Core modules for the Realtime platform.

pub mod api;
mod capabilities;
mod error;

#[cfg(target_arch = "wasm32")]
pub use guest_macro::*;
pub use {anyhow, axum, bytes, fromenv, http, http_body, tracing};
#[cfg(target_arch = "wasm32")]
pub use {wasi_http, wasi_identity, wasi_keyvalue, wasi_messaging, wasi_otel, wasip3, wit_bindgen};

pub use crate::api::*;
pub use crate::capabilities::*;
pub use crate::error::*;
