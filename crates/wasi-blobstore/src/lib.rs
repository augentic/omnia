#![doc = include_str!("../README.md")]

//! # WASI Blobstore Service
//!
//! This module implements a runtime service for `omnia:blobstore`.

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
