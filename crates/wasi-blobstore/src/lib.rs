#![doc = include_str!("../README.md")]

//! # WASI Blobstore Service
//!
//! This module implements a runtime service for `wasi:blobstore`
//! (<https://github.com/WebAssembly/wasi-blobstore>).

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
