#![doc = include_str!("../README.md")]

//! # WASI SQL Service
//!
//! This module implements a runtime service for `wasi:sql`
//! (<https://github.com/aspect-build/wasi-sql>).

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
