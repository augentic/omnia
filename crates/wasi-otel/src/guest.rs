//! # Guest
//!
//! The guest half of `omnia:otel`: a `tracing` subscriber whose spans and
//! metrics reach the host through the crate's private `wasi:otel` bindings.

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
