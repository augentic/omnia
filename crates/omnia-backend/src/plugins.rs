//! Plugin package store service contract and reference type.

#[cfg(not(target_arch = "wasm32"))]
mod client;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{PackageStore, Reference};
