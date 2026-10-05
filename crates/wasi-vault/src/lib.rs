#![doc = include_str!("../README.md")]

//! # WASI Vault Service
//!
//! This module implements a runtime service for `wasi:vault`
//! (<https://github.com/augentic/wasi-vault>).

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
