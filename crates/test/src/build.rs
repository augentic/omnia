//! The fixture pipeline a consumer's `build.rs` drives.
//!
//! Guest programs are compiled to `wasm32-wasip2` components in a nested
//! cargo build, then `gen.rs` is generated with one path constant per
//! program plus a `foreach_<group>!` completeness macro per group.
//!
//! ```no_run
//! // build.rs
//! omnia_test::build::Components::in_workspace("../..")
//!     .package("test-programs")
//!     .scan("crates/test-programs/programs")
//!     .sync_examples("crates/test-programs/Cargo.toml")
//!     .track(["crates/test-programs/wit"])
//!     .build()
//!     .write_gen("gen.rs");
//! ```
//!
//! The nested build is a no-op when the outer target is itself `wasm32`;
//! `gen.rs` then holds only its header.

mod components;
mod env;
mod render;

pub use components::{Built, Components, EXAMPLES_MARKER, Program};
