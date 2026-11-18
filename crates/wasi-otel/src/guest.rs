//! # Guest
//!
//! The guest half of `omnia:otel`: a `tracing` subscriber whose spans and
//! metrics reach the host through the [`omnia`] bindings.

mod aggregate;
mod convert;
mod init;
mod metrics;
mod tracing;

mod generated {
    #![allow(
        missing_docs,
        clippy::future_not_send,
        clippy::collection_is_never_read,
        clippy::same_length_and_capacity,
        reason = "wit-bindgen output"
    )]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:otel`, the interfaces behind the guest telemetry library.
pub mod omnia {
    pub use crate::guest::generated::omnia::otel::*;
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
