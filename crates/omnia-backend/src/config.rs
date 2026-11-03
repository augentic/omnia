//! Configuration lookup service contract and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "config-default"))]
mod default;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::ConfigClient;
#[cfg(all(not(target_arch = "wasm32"), feature = "config-default"))]
pub use self::default::ConfigDefault;
