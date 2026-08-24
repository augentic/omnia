//! Guest–host ABI tests for the Omnia workspace, and their scaffolding.
//!
//! The suite lives in `tests/`, one integration-test target per scenario
//! family, run process-per-test under Nextest with the rest of the workspace.
//! Build the guests it drives with `cargo make test-guests` (`cargo make
//! test` does both).
//!
//! The library is the shared scaffolding:
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

pub mod http;

mod guest;
mod manifest;
mod runtime;

pub use self::guest::{find_guest, precompiled_artifact, precompiled_bytes, raw_wasm, wasm_bytes};
pub use self::manifest::{TempManifest, temp_manifest};
pub use self::runtime::{SingleGuest, single_guest};
