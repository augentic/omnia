#![doc = include_str!("../README.md")]

//! # WASI Model
//!
//! This module implements the runtime boundary for `omnia:model/completion`:
//! a guest calls `create` to have a prompt completed and receives a validated
//! typed answer, without ever seeing which backend produced it.

pub mod prompt;

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub use guest::*;

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
