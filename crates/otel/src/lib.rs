#![doc = include_str!("../README.md")]

//! # Telemetry
//!
//! Telemetry is a module that provides functionality for collecting and
//! reporting OpenTelemetry-based metrics.

#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

pub mod init;
pub mod tracing;

pub use init::Telemetry;
pub use tracing::*;
