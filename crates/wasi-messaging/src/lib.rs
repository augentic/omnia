#![doc = include_str!("../README.md")]

//! # WASI Messaging
//!
//! This module implements a runtime service for `omnia:messaging`.

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
