#![doc = include_str!("../README.md")]

//! # WASI `DocStore`
//!
//! This module implements a runtime service for `wasi:docstore`: a JSON
//! document store with a backend-portable filter language.

pub mod document_store;

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
