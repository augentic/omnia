#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI `DocStore`
//!
//! This module implements a runtime service for `omnia:docstore`: a JSON
//! document store with a backend-portable filter language.

mod host;
pub use host::*;
