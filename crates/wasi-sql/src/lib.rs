#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI SQL Service
//!
//! This module implements a runtime service for `omnia:sql`.

mod host;
pub use host::*;
