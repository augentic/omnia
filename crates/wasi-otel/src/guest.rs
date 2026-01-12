//! # WASI Bindings
//!
//! This module generates and exports WASI Guest bindings for local wit worlds.
//! The bindings are exported in as similar a manner to those in the Bytecode
//! Alliance's [wasi] crate.
//!
//! [wasi]: https://github.com/bytecodealliance/wasi

#![allow(clippy::same_length_and_capacity)]

mod convert;
mod init;
#[cfg(feature = "metrics")]
mod metrics;
#[cfg(feature = "tracing")]
mod tracing;

// Bindings for the `wasi:otel` world.
mod generated {
    #![allow(clippy::future_not_send)]
    #![allow(clippy::collection_is_never_read)]

    wit_bindgen::generate!({
        world: "otel",
        path: "wit",
        generate_all,
    });
}

/// Re-exported `instrument` macro for use in guest code.
pub use yetti_wasi_otel_attr::instrument;

use self::init::ExitGuard;
use crate::guest::init::INIT;

/// Initialize OpenTelemetry SDK and tracing subscriber.
pub fn init() -> Option<ExitGuard> {
    // if init::INIT.read().is_ok_and(|x| *x) {
    //     return None;
    // }
    if INIT.get().is_some() {
        return None;
    }

    match init::init() {
        Ok(guard) => Some(guard),
        Err(e) => {
            ::tracing::error!("failed to initialize: {e}");
            None
        }
    }
}
