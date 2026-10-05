#![doc = include_str!("../README.md")]

//! # WASI Config Service
//!
//! This module implements a runtime service for `wasi:config`
//! (<https://github.com/WebAssembly/wasi-config>).

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
