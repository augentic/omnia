#![doc = include_str!("../README.md")]

//! # WASI SQL Service
//!
//! This module implements a runtime service for `wasi:sql`
//! (<https://github.com/aspect-build/wasi-sql>).

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
