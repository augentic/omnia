//! `omnia:otel`, and the guest telemetry library over it.
//!
//! On `wasm32`, `scope` installs a `tracing` subscriber whose spans and
//! metrics reach the host through [`traces`] and [`metrics`], and
//! [`instrument`] runs an `async fn` inside that scope.
//!
//! The unstable [`logs`] interface is the raw binding. Every host links it,
//! and answers `unsupported` today.

pub use crate::bindings::imports::omnia::otel::{logs, metrics, traces, types};

cfg_select! {
    target_arch = "wasm32" => {
        mod aggregate;
        mod convert;
        mod init;
        mod meter;
        mod tracer;

        pub use self::init::*;
    }
    _ => {}
}

/// Opens a span around a fn, and runs an `async fn` inside the telemetry scope.
pub use omnia_guest_macros::instrument;

// Implementation detail of the `#[instrument]` expansion: the macro emits
// paths through this module so callers need no direct `tracing` dependency.
#[doc(hidden)]
pub mod __private {
    pub use ::tracing;
}
