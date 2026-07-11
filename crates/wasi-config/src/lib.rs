#![doc = include_str!("../README.md")]

//! # WASI Config Service
//!
//! This module implements a runtime service for `wasi:config`
//! (<https://github.com/WebAssembly/wasi-config>).

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
