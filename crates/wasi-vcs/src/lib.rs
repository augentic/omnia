#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

//! # WASI VCS
//!
//! This module implements the runtime boundary for `omnia:vcs`: a guest
//! opens a workspace at a location beneath its mounts, and the host carries
//! each call to a version-control backend, which works through an open
//! directory handle beneath the mount and never sees a descriptor.

mod host;
pub use host::*;
