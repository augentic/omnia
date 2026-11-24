//! `omnia:otel`, and the guest telemetry library over it.
//!
//! [`scope`] installs a `tracing` subscriber whose spans and metrics reach
//! the host through [`traces`] and [`metrics`], and [`flush`] exports what
//! it has buffered. [`instrument`] runs an `async fn` inside that scope. On a
//! native target `scope` installs nothing, so spans reach whatever subscriber
//! the caller set.
//!
//! The unstable [`logs`] interface is the raw binding. Every host links it,
//! and answers `unsupported` today.

mod aggregate;
mod convert;
mod init;
mod meter;
mod tracer;

/// Opens a span around a fn, and runs an `async fn` inside the telemetry scope.
pub use omnia_guest_macros::instrument;

pub use self::init::{flush, scope};
pub use crate::bindings::imports::omnia::otel::*;

// Implementation detail of the `#[instrument]` expansion: the macro emits
// paths through this module so callers need no direct `tracing` dependency.
#[doc(hidden)]
pub mod __private {
    pub use ::tracing;
}
