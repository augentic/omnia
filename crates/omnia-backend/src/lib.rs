#![doc = include_str!("../README.md")]

#[cfg(feature = "blobstore")]
pub mod blobstore;
#[cfg(feature = "config")]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
mod connect;
#[cfg(feature = "docstore")]
pub mod docstore;
#[cfg(feature = "http")]
pub mod http;
#[cfg(feature = "identity")]
pub mod identity;
#[cfg(feature = "keyvalue")]
pub mod keyvalue;
#[cfg(feature = "messaging")]
pub mod messaging;
#[cfg(feature = "model")]
pub mod model;
#[cfg(not(target_arch = "wasm32"))]
mod mount;
#[cfg(feature = "otel")]
pub mod otel;
#[cfg(feature = "plugins")]
pub mod plugins;
#[cfg(feature = "sql")]
pub mod sql;
#[cfg(all(not(target_arch = "wasm32"), feature = "otel"))]
pub mod telemetry;
#[cfg(feature = "vault")]
pub mod vault;
#[cfg(feature = "vcs")]
pub mod vcs;
#[cfg(feature = "websocket")]
pub mod websocket;

#[cfg(all(not(target_arch = "wasm32"), feature = "otel"))]
pub use tracing_subscriber::filter::LevelFilter;

#[cfg(all(not(target_arch = "wasm32"), feature = "blobstore"))]
pub use self::blobstore::{BlobStoreClient, BlobStoreDefault, Container};
#[cfg(all(not(target_arch = "wasm32"), feature = "config"))]
pub use self::config::{ConfigClient, ConfigDefault};
#[cfg(not(target_arch = "wasm32"))]
pub use self::connect::{Connect, FromEnv, FutureResult, NoOptions};
#[cfg(all(not(target_arch = "wasm32"), feature = "docstore"))]
pub use self::docstore::{DocStoreClient, DocStoreDefault};
#[cfg(all(not(target_arch = "wasm32"), feature = "http"))]
pub use self::http::{HttpClient, HttpDefault};
#[cfg(all(not(target_arch = "wasm32"), feature = "oauth"))]
pub use self::identity::IdentityDefault;
#[cfg(all(not(target_arch = "wasm32"), feature = "identity"))]
pub use self::identity::{Identity, IdentityClient, IdentityStub};
#[cfg(all(not(target_arch = "wasm32"), feature = "keyvalue"))]
pub use self::keyvalue::{Bucket, KeyValueClient, KeyValueDefault};
#[cfg(all(not(target_arch = "wasm32"), feature = "messaging"))]
pub use self::messaging::{
    Endpoint as MessagingEndpoint, Handler, MessagingClient, MessagingDefault,
};
#[cfg(all(not(target_arch = "wasm32"), feature = "model"))]
pub use self::model::{
    Answer, Limits, ModelClient, ModelDefault, ToolHost, ToolSession, Workspace,
};
#[cfg(not(target_arch = "wasm32"))]
pub use self::mount::{Mount, MountRegistry, ResolvedPreopen};
#[cfg(all(not(target_arch = "wasm32"), feature = "otel"))]
pub use self::otel::{OtelClient, OtelDefault};
#[cfg(all(not(target_arch = "wasm32"), feature = "plugins"))]
pub use self::plugins::{PackageStore, Reference};
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
pub use self::sql::{ConnectOptions as SqlConnectOptions, SqlDefault};
#[cfg(all(not(target_arch = "wasm32"), feature = "sql"))]
pub use self::sql::{Connection, SqlClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "otel"))]
pub use self::telemetry::Telemetry;
#[cfg(all(not(target_arch = "wasm32"), feature = "vault"))]
pub use self::vault::{Locker, VaultClient, VaultDefault};
#[cfg(all(not(target_arch = "wasm32"), feature = "vcs"))]
pub use self::vcs::{Intent, Place, VcsClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "websocket"))]
pub use self::websocket::{Endpoint as WebSocketEndpoint, WebSocketClient, WebSocketDefault};
