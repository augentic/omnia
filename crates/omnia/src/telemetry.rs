//! # Telemetry
//!
//! Host-side OpenTelemetry initialization, OTLP exporters, and the `tracing`
//! middleware used to report runtime telemetry out-of-the-box.

mod init;
mod tracing;

pub use init::*;
pub use tracing::*;
