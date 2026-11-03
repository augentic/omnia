//! Provider capabilities: the host services a guest's provider can call.
//!
//! Each capability is a target-neutral trait implemented by [`WasiProvider`] on
//! `wasm32` (delegating to the matching `omnia-wasi-*` binding), by
//! `omnia_native::Direct` in-process, and by `omnia_test::guest` doubles in
//! tests.
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

pub use blob::{BlobStore, ContainerMetadata, HasBlobStore, ObjectMetadata};
pub use broadcast::{Broadcast, HasBroadcast};
pub use config::{Config, HasConfig};
#[cfg(feature = "docstore")]
pub use document::{DocumentStore, HasDocumentStore};
pub use http::{HasHttpRequest, HttpRequest};
pub use identity::{AccessToken, HasIdentity, Identity};
pub use messaging::{HasPublish, Message, Publish};
// Generic model wire names stay scoped to the model capability.
pub use model::{HasModel, Model};
// Loader request names stay scoped to the plugins capability.
pub use plugins::{HasPlugins, Plugins};
pub use state::{CasError, HasStateStore, StateStore};
#[cfg(feature = "sql")]
pub use table::{HasTableStore, TableStore};
pub use vault::{HasVault, Vault};
// Version-control wire names stay scoped to the vcs capability.
pub use vcs::{HasVcs, Vcs};

/// Zero-sized WASI provider implementing every capability trait on `wasm32`.
///
/// Delegates to the `omnia-wasi-*` guest bindings on `wasm32-wasip2`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WasiProvider;
