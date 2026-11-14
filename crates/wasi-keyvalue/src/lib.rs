#![doc = include_str!("../README.md")]

//! # WASI Key-Value
//!
//! This module implements a runtime service for `omnia:keyvalue` and the
//! `wasi:keyvalue` draft (<https://github.com/WebAssembly/wasi-keyvalue>).

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
