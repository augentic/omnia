#![doc = include_str!("../README.md")]

#[cfg(feature = "blobstore")]
pub mod blobstore;
#[cfg(feature = "config")]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
mod connect;
#[cfg(feature = "docstore")]
pub mod docstore;
#[cfg(feature = "identity")]
pub mod identity;
#[cfg(feature = "keyvalue")]
pub mod keyvalue;
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

#[cfg(all(not(target_arch = "wasm32"), feature = "blobstore"))]
pub use self::blobstore::{BlobStoreClient, Container};
#[cfg(all(not(target_arch = "wasm32"), feature = "config"))]
pub use self::config::ConfigClient;
#[cfg(not(target_arch = "wasm32"))]
pub use self::connect::{Connect, FromEnv, FutureResult, NoOptions};
#[cfg(all(not(target_arch = "wasm32"), feature = "docstore"))]
pub use self::docstore::DocStoreClient;
#[cfg(all(not(target_arch = "wasm32"), feature = "identity"))]
pub use self::identity::{Identity, IdentityClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "keyvalue"))]
pub use self::keyvalue::{Bucket, KeyValueClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "messaging"))]
pub use self::messaging::{Endpoint as MessagingEndpoint, Handler, MessagingClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "model"))]
pub use self::model::{Answer, Limits, ModelClient, ToolHost};
pub use self::mount::{check_subpath, lend};
#[cfg(all(not(target_arch = "wasm32"), feature = "otel"))]
pub use self::otel::OtelClient;
#[cfg(all(not(target_arch = "wasm32"), feature = "plugins"))]
pub use self::plugins::{PackageStore, Reference};
#[cfg(all(not(target_arch = "wasm32"), feature = "sql"))]
pub use self::sql::{Connection, SqlClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "vault"))]
pub use self::vault::{Locker, VaultClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "vcs"))]
pub use self::vcs::{Place, VcsClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "websocket"))]
pub use self::websocket::{Endpoint as WebSocketEndpoint, WebSocketClient};
