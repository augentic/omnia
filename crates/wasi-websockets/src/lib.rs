//! # WASI Websockets Service
//!
//! This module implements a runtime service for `wasi:websockets`
//! (<https://github.com/augentic/wasi-websockets>).

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
