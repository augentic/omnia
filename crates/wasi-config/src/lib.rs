#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI Config Service
//!
//! This module implements a runtime service for `omnia:config`.

mod host;
pub use host::*;
