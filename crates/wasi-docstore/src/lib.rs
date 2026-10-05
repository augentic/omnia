#![doc = include_str!("../README.md")]

//! # WASI `DocStore`
//!
//! This module implements a runtime service for `wasi:docstore`: a JSON
//! document store with a backend-portable filter language.

pub mod document_store;

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
