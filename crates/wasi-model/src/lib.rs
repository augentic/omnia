#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI Model
//!
//! This module implements the runtime boundary for `omnia:model`: a guest
//! opens a model the deployment declares and calls `complete` to have a
//! prompt completed. It receives an answer it may have judged itself, without
//! ever seeing which backend produced it.

mod host;
pub use host::*;
