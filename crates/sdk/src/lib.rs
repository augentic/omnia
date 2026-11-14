#![doc = include_str!("../README.md")]

pub mod api;
mod capabilities;
mod error;
#[cfg(feature = "http")]
pub mod mcp;

pub use omnia_guest_macros::{Provider, Settings, capability};

pub use crate::capabilities::*;
pub use crate::error::*;

// WASI provider implementation.
cfg_select! {
    all(target_arch = "wasm32", target_os = "wasi") => {
        pub mod wasi;

        #[doc(hidden)]
        pub use {omnia_wasi_messaging, wasip3};

        pub use crate::wasi::WasiProvider;
    }
    _ => {}
}
