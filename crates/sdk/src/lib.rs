#![doc = include_str!("../README.md")]

pub mod api;
mod capabilities;
mod error;
#[cfg(feature = "http")]
pub mod mcp;
pub mod wasi;

pub use omnia_guest_macros::{Provider, Settings, capability};
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
pub use {omnia_wasi_messaging, wasip3};

pub use crate::capabilities::{capability, *};
pub use crate::error::*;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub use crate::wasi::WasiProvider;
