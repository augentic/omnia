#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI Vault
//!
//! This module implements a runtime service for `omnia:vault`.

mod host;
pub use host::*;
