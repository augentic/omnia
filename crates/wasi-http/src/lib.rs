#![doc = include_str!("../README.md")]

//! # WASI Http Service
//!
//! This module implements a runtime service for `wasi:http`
//! (<https://github.com/WebAssembly/wasi-http>).

#![forbid(unsafe_code)]

/// Per-request resilience policy for outbound HTTP.
///
/// Attach to a request via extensions before calling
/// [`omnia_sdk::HttpRequest::fetch`]. The guest runtime serializes this into
/// internal headers that the host reads and strips — the upstream never sees
/// them.
///
/// ```rust,ignore
/// use omnia_sdk::{HttpRequest, OutboundPolicy};
///
/// let request = http::Request::builder()
///     .uri("https://api.example.com/data")
///     .extension(OutboundPolicy {
///         timeout_ms: Some(5000),
///         upstream: Some("my-service".into()),
///     })
///     .body(Empty::<Bytes>::new())?;
///
/// let response = HttpRequest::fetch(&provider, request).await?;
/// ```
#[derive(Clone, Debug, Default)]
pub struct OutboundPolicy {
    /// Response timeout in milliseconds. Falls back to host default if `None`.
    pub timeout_ms: Option<u64>,
    /// Override breaker bucket name. Falls back to the default breaker if `None`.
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
