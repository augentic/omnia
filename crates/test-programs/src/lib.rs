//! The guest scenario programs the e2e suites drive, from both sides of the
//! boundary.
//!
//! On `wasm32` the crate is the programs' shared helpers; each program under
//! `programs/<capability>/<scenario>.rs` is an `[[example]]` compiled to a
//! component. Natively it is the compiled artifacts: `build.rs` runs that
//! `wasm32-wasip2` build and generates one `pub const <NAME>: &str` path per
//! program plus a `foreach_<capability>!` macro a suite invokes to prove
//! every program has a matching test. A suite runs an artifact through
//! `omnia_test::host`.

#[cfg(target_arch = "wasm32")]
mod helpers;

#[cfg(target_arch = "wasm32")]
pub use helpers::*;

#[cfg(not(target_arch = "wasm32"))]
include!(concat!(env!("OUT_DIR"), "/gen.rs"));
