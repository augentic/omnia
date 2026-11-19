#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI Messaging
//!
//! This module implements a runtime service for `omnia:messaging`.

mod host;
pub use host::*;
