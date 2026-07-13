//! Shared scaffolding for testing Omnia guests and runtimes.
//!
//! The lightweight [`model`] helpers exercise model-consuming core logic
//! without constructing a Wasmtime runtime. Runtime helpers remain available
//! through the default `runtime` feature:
//!
//! - [`find_guest`] locates a pre-built example guest artifact and fails fast
//!   when it is missing. Tests never invoke Cargo: build guests up front with
//!   `cargo make test-guests`.
//! - [`temp_manifest`] writes a deployment manifest to a unique temp file and
//!   removes it on drop.
//! - [`single_guest`] assembles a single-guest [`omnia::Runtime`] over a
//!   backend bundle, absorbing the deployment/link/registry boilerplate.
//! - [`http`] drives a guest's `wasi:http/handler` export in-process, without
//!   binding a TCP socket.

#![cfg(not(target_arch = "wasm32"))]

#[cfg(feature = "runtime")]
pub mod http;
#[cfg(feature = "model")]
pub mod model;

#[cfg(feature = "runtime")]
mod guest;
#[cfg(feature = "runtime")]
mod manifest;
#[cfg(feature = "runtime")]
mod runtime;

#[cfg(feature = "runtime")]
pub use self::guest::find_guest;
#[cfg(feature = "runtime")]
pub use self::manifest::{TempManifest, temp_manifest};
#[cfg(feature = "runtime")]
pub use self::runtime::{SingleGuest, single_guest};
