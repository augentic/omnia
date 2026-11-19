#![doc = include_str!("../README.md")]

//! # WASI Types
//!
//! This module holds the guest bindings for `omnia:types`.

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;
