#![doc = include_str!("../README.md")]

//! # WASI OpenTelemetry
//!
//! Bindings for `omnia:otel`, the OpenTelemetry interface, for guest and host
//! components.

cfg_select! {
    target_arch = "wasm32" => {
        mod guest;
        pub use guest::*;
    }
    _ => {
        mod host;
        pub use host::*;
    }
}
