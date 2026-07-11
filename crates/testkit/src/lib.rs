//! Shared scaffolding for Omnia integration tests.
//!
//! Every WASI interface is exercised end-to-end by a pre-built guest `.wasm`
//! driven through the real runtime. This crate factors out the mechanics each
//! such test would otherwise duplicate:
//!
//! - [`find_guest`] locates a pre-built example guest artifact and fails fast
//!   when it is missing. Tests never invoke Cargo: build guests up front with
//!   `cargo make build-test-guests`.
//! - [`temp_manifest`] writes a deployment manifest to a unique temp file and
//!   removes it on drop.
//! - [`single_guest`] assembles a single-guest [`omnia::Runtime`] over a
//!   backend bundle, absorbing the deployment/link/registry boilerplate.
//! - [`http`] drives a guest's `wasi:http/handler` export in-process, without
//!   binding a TCP socket.

#![cfg(not(target_arch = "wasm32"))]

pub mod http;

mod guest;
mod manifest;
mod runtime;

pub use self::guest::find_guest;
pub use self::manifest::{TempManifest, temp_manifest};
pub use self::runtime::{SingleGuest, single_guest};
