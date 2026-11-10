#![doc = include_str!("../README.md")]

#[cfg(feature = "blobstore")]
pub mod blobstore;
#[cfg(not(target_arch = "wasm32"))]
mod capability;
#[cfg(feature = "config")]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
mod connect;
#[cfg(feature = "docstore")]
pub mod docstore;
#[cfg(feature = "identity")]
pub mod identity;
#[cfg(feature = "messaging")]
pub mod messaging;
#[cfg(feature = "model")]
pub mod model;
mod mount;
#[cfg(feature = "otel")]
pub mod otel;
#[cfg(feature = "plugins")]
pub mod plugins;
#[cfg(feature = "sql")]
pub mod sql;
#[cfg(feature = "vault")]
pub mod vault;
#[cfg(feature = "vcs")]
pub mod vcs;
#[cfg(feature = "websocket")]
pub mod websocket;

#[cfg(not(target_arch = "wasm32"))]
pub use self::capability::{Backend, Capability, Serve, Serves};
#[cfg(not(target_arch = "wasm32"))]
pub use self::connect::{Connect, FromEnv, FutureResult, NoOptions};
#[cfg(not(target_arch = "wasm32"))]
pub use self::mount::check_subpath;
pub use self::mount::lend;
