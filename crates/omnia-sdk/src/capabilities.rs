//! Provider capabilities: the host services a guest's provider can call.
//!
//! Each capability is a target-neutral trait implemented by [`WasiProvider`] on
//! `wasm32` (delegating to the matching `omnia-wasi-*` binding), by
//! `omnia_test::guest` doubles in native handler tests, and by capability
//! decorators composed via `#[derive(Provider)]`.
//!
//! Every capability has a companion `Has<Capability>` accessor trait (emitted
//! by `#[capability]`) with a blanket forwarding impl and implementations for
//! `Arc<P>`, `&P`, and `Box<P>`, so `#[derive(Provider)]` and pointer
//! wrappers delegate without repeating method signatures.
//!
//! A `<Capability>Ext` trait ([`StateStoreExt`], [`ConfigExt`], ...) holds the
//! caller-side codecs and parsers over a capability's primitives. It is
//! blanket-implemented for every implementor and never implemented by hand, so
//! one encoding holds across providers. Its methods return
//! [`capability::Result`](crate::capability::Result), with a codec failure as
//! [`ErrorKind::Invalid`](crate::capability::ErrorKind::Invalid).
//! [`HttpClientExt::fetch_json`] alone returns [`crate::Result`], since it
//! also maps a non-2xx status to [`Error::BadGateway`](crate::Error::BadGateway).

mod blob;
mod config;
mod document;
mod http;
mod identity;
mod lend;
mod messaging;
pub mod model;
pub mod plugins;
mod secrets;
mod sql;
mod state;
pub mod vcs;
mod websockets;

pub use blob::BlobStore;
#[doc(hidden)]
pub use blob::HasBlobStore;
#[doc(hidden)]
pub use config::HasConfig;
pub use config::{Config, ConfigExt};
#[doc(hidden)]
pub use document::HasDocumentDb;
pub use document::{DocumentDb, DocumentDbExt};
#[doc(hidden)]
pub use http::HasHttpClient;
pub use http::{HttpClient, HttpClientExt};
#[doc(hidden)]
pub use identity::HasIdentity;
pub use identity::{Identity, IdentityExt};
#[doc(hidden)]
pub use messaging::HasPublisher;
pub use messaging::Publisher;
#[doc(hidden)]
pub use model::HasModel;
// Generic model wire names stay scoped to the model capability.
pub use model::Model;
#[doc(hidden)]
pub use plugins::HasPlugins;
// Loader request names stay scoped to the plugins capability.
pub use plugins::Plugins;
#[doc(hidden)]
pub use secrets::HasSecretStore;
pub use secrets::SecretStore;
#[doc(hidden)]
pub use sql::HasSqlDb;
pub use sql::{SqlDb, SqlDbExt};
#[doc(hidden)]
pub use state::HasStateStore;
pub use state::{StateStore, StateStoreExt};
#[doc(hidden)]
pub use vcs::HasVcs;
// Version-control wire names stay scoped to the vcs capability.
pub use vcs::Vcs;
#[doc(hidden)]
pub use websockets::HasWebSockets;
pub use websockets::WebSockets;

/// Zero-sized WASI provider implementing every capability trait on `wasm32`.
///
/// Delegates to the `omnia-wasi-*` guest bindings on `wasm32-wasip2`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WasiProvider;
