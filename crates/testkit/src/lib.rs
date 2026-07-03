//! Shared scaffolding for Omnia integration tests.
//!
//! Every WASI interface is exercised end-to-end by a pre-built guest `.wasm`
//! driven through the real runtime. This crate factors out the mechanics each
//! such test would otherwise duplicate:
//!
//! - [`find_guest`] locates a built example guest (building guests on first use),
//!   encoding the "fail in CI, skip locally" policy so a missing guest never lets
//! - [`temp_manifest`] writes a deployment manifest to a unique temp file and
//!   removes it on drop.
//! - [`http`] drives a guest's `wasi:http/handler` export in-process, without
//!   binding a TCP socket.
//!
//! Guests are built on first [`find_guest`] call.

#![cfg(not(target_arch = "wasm32"))]

pub mod http;

mod guest;
mod manifest;

pub use self::guest::find_guest;
pub use self::manifest::{TempManifest, temp_manifest};
