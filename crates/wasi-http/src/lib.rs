#![doc = include_str!("../README.md")]

//! # WASI Http Service
//!
//! This module implements a runtime service for `wasi:http`
//! (<https://github.com/WebAssembly/wasi-http>).

#![forbid(unsafe_code)]

/// Per-request resilience policy for outbound HTTP.
///
/// Attach to a request via extensions before calling `fetch` method of `HttpRequest` trait.
/// The guest runtime serializes this into internal headers that the host reads
/// and strips — the upstream never sees them.
///
/// Re-exported as `omnia_sdk::OutboundPolicy` for convenience.
#[derive(Clone, Debug, Default)]
pub struct OutboundPolicy {
    /// Response timeout in milliseconds. Falls back to host default if `None`.
    pub timeout_ms: Option<u64>,
    /// Circuit breaker bucket name. When `None`, the breaker is bypassed entirely.
    pub upstream: Option<String>,
}

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
