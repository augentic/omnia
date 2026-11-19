#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI WebSocket Service
//!
//! This module implements a runtime service for `omnia:websocket`.

mod host;
pub use host::*;
