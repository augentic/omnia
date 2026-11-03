//! Outbound HTTP service contract and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "http-default"))]
pub mod client_cert;
#[cfg(all(not(target_arch = "wasm32"), feature = "http-default"))]
mod default;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Body, Error, HttpClient, RequestOptions};
#[cfg(all(not(target_arch = "wasm32"), feature = "http-default"))]
pub use self::default::{ConnectOptions, HttpDefault};
