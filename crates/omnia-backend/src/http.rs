//! Outbound HTTP service contract.

#[cfg(not(target_arch = "wasm32"))]
mod client;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Body, Error, HttpClient, RequestOptions};
