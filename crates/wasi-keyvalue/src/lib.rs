#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI Key-Value
//!
//! This module implements a runtime service for `omnia:keyvalue`.

mod host;
pub use host::*;
