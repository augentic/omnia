#![doc = include_str!("../README.md")]

//! # WASI Model
//!
//! This module implements the runtime boundary for `omnia:model`: a guest
//! opens a model the deployment declares and calls `complete` to have a
//! prompt completed. It receives an answer it may have judged itself, without
//! ever seeing which backend produced it. The optional `mcp` interface offers
//! Model Context Protocol servers' tools beside the guest's own.

pub mod prompt;

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
