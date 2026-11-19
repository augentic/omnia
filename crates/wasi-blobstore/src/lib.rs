#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI Blobstore Service
//!
//! This module implements a runtime service for `omnia:blobstore`.

mod host;
pub use host::*;
