#![doc = include_str!("../README.md")]

//! # WASI WebSocket Service
//!
//! This module implements a runtime service for `wasi:websocket`
//! (<https://github.com/augentic/wasi-websocket>).

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
