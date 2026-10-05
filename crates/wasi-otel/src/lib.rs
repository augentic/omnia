#![doc = include_str!("../README.md")]

//! # WASI OpenTelemetry
//!
//! Bindings for the OpenTelemetry specification (wasi:otel) for guest and host
//! components.

mod trace_state;

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
