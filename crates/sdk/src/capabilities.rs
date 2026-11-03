//! Provider capabilities: the host services a guest's provider can call.
//!
//! Each capability is a target-neutral trait implemented by [`WasiProvider`] on
//! `wasm32` (delegating to the matching `omnia-wasi-*` binding), by
//! `omnia_test::guest` doubles in native handler tests, and by capability
//! decorators composed via `#[derive(Provider)]`.
//!
//! Every capability has a companion `Has<Capability>` accessor trait (emitted
//! by `#[omnia_guest_macros::capability]`) with a blanket forwarding impl and
//! implementations for `Arc<P>`, `&P`, and `Box<P>`, so `#[derive(Provider)]`
//! and pointer wrappers delegate without repeating method signatures.

mod blob;
mod broadcast;
mod config;
#[cfg(feature = "docstore")]
mod document;
mod http;
mod identity;
mod lend;
mod messaging;
pub mod model;
pub mod plugins;
mod state;
#[cfg(feature = "sql")]
mod table;
mod vault;
pub mod vcs;

#[doc(hidden)]
pub use blob::HasBlobStore;
pub use blob::{BlobStore, ContainerMetadata, ObjectMetadata};
pub use broadcast::Broadcast;
#[doc(hidden)]
pub use broadcast::HasBroadcast;
pub use config::Config;
#[doc(hidden)]
pub use config::HasConfig;
#[cfg(feature = "docstore")]
pub use document::DocumentStore;
#[cfg(feature = "docstore")]
#[doc(hidden)]
pub use document::HasDocumentStore;
#[doc(hidden)]
pub use http::HasHttpRequest;
pub use http::HttpRequest;
#[doc(hidden)]
pub use identity::HasIdentity;
pub use identity::{AccessToken, Identity};
#[doc(hidden)]
pub use messaging::HasPublish;
pub use messaging::{Message, Publish};
#[doc(hidden)]
pub use model::HasModel;
// Generic model wire names stay scoped to the model capability.
pub use model::Model;
#[doc(hidden)]
pub use plugins::HasPlugins;
// Loader request names stay scoped to the plugins capability.
pub use plugins::Plugins;
#[doc(hidden)]
pub use state::HasStateStore;
pub use state::{CasError, StateStore};
#[cfg(feature = "sql")]
#[doc(hidden)]
pub use table::HasTableStore;
#[cfg(feature = "sql")]
pub use table::TableStore;
#[doc(hidden)]
pub use vault::HasVault;
pub use vault::Vault;
#[doc(hidden)]
pub use vcs::HasVcs;
// Version-control wire names stay scoped to the vcs capability.
pub use vcs::Vcs;

/// Zero-sized WASI provider implementing every capability trait on `wasm32`.
///
/// Delegates to the `omnia-wasi-*` guest bindings on `wasm32-wasip2`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WasiProvider;
