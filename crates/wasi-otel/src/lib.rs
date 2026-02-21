#![doc = include_str!("../README.md")]

//! # WASI OpenTelemetry
//!
//! Bindings for the OpenTelemetry specification (wasi:otel) for guest and host
//! components.

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
