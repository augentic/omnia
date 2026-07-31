//! # WASI Bindings
//!
//! This module generates and exports WASI Guest bindings for local wit worlds.
//! The bindings are exported in as similar a manner to those in the Bytecode
//! Alliance's [wasi] crate.
//!
//! [wasi]: https://github.com/bytecodealliance/wasi

mod convert;
mod init;
mod metrics;
mod tracing;

// Bindings for the `wasi:otel` world.
mod generated {
    #![allow(clippy::future_not_send)]
    #![allow(clippy::collection_is_never_read)]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// Re-exported `instrument` macro for use in guest code.
pub use omnia_guest_macros::instrument;

pub use crate::guest::init::*;

// Implementation detail of the `#[instrument]` expansion: the macro emits
// paths through this module so callers need no direct `tracing` dependency.
#[doc(hidden)]
pub mod __private {
    pub use ::tracing;
}
