//! OpenTelemetry exporter service contract and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "otel-default"))]
mod default;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::OtelClient;
#[cfg(all(not(target_arch = "wasm32"), feature = "otel-default"))]
pub use self::default::OtelDefault;
