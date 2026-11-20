#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI OpenTelemetry
//!
//! Bindings for `omnia:otel`, the OpenTelemetry interface, for guest and host
//! components.

mod host;
pub use host::*;
