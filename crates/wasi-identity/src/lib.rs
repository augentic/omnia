#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI Identity Service
//!
//! This module implements a runtime service for `omnia:identity`.

mod host;
pub use host::*;
