#![doc = include_str!("../README.md")]

//! # WASI WebSocket Service
//!
//! This module implements a runtime service for `wasi:websocket`
//! (<https://github.com/augentic/wasi-websocket>).

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
