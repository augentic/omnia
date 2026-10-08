#![doc = include_str!("../README.md")]

//! # WASI VCS
//!
//! This module implements the runtime boundary for `omnia:vcs`: a guest
//! names repositories and working copies as locations beneath its mounts
//! and the host carries each operation to a version-control backend, which
//! works on the resolved host paths and never sees a descriptor.

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
