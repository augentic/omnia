#![doc = include_str!("../README.md")]

//! # WASI `DocStore`
//!
//! This module implements a runtime service for `omnia:docstore`: a JSON
//! document store with a backend-portable filter language.

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
