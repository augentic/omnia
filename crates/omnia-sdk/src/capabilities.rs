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
//!
//! A `<Capability>Ext` trait ([`StateStoreExt`], [`ConfigExt`], ...) holds the
//! caller-side codecs and parsers over a capability's primitives. It is
//! blanket-implemented for every implementor and never implemented by hand, so
//! one encoding holds across providers. Its methods return the error type of
//! the primitives they wrap:
//!
//! - [`StateStoreExt`] returns [`capability::Result`](crate::capability::Result),
//!   with a codec failure as [`ErrorKind::Invalid`](crate::capability::ErrorKind::Invalid).
//! - [`HttpRequestExt::fetch_json`] returns [`crate::Result`], since it also
//!   maps a non-2xx status to [`Error::BadGateway`](crate::Error::BadGateway).
//! - The others return `anyhow::Result`.
//!
//! [`Capabilities`] is every capability trait at once, the bound a fn generic
//! over the whole provider takes.

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
#[doc(hidden)]
pub use config::HasConfig;
pub use config::{Config, ConfigExt};
#[cfg(feature = "docstore")]
#[doc(hidden)]
pub use document::HasDocumentStore;
#[cfg(feature = "docstore")]
pub use document::{DocumentStore, DocumentStoreExt};
#[doc(hidden)]
pub use http::HasHttpRequest;
pub use http::{HttpRequest, HttpRequestExt};
#[doc(hidden)]
pub use identity::HasIdentity;
pub use identity::{AccessToken, Identity, IdentityExt};
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
pub use state::{StateStore, StateStoreExt};
#[cfg(feature = "sql")]
#[doc(hidden)]
pub use table::HasTableStore;
#[cfg(feature = "sql")]
pub use table::{TableStore, TableStoreExt};
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

// `#[cfg]` cannot gate one supertrait, so each feature set names the traits
// it adds to the always-on list.
macro_rules! capabilities {
    ($($gated:ident),*) => {
        /// Every capability trait at once, with the `Send + Sync + 'static` a
        /// provider needs.
        ///
        /// A fn generic over the whole provider takes this bound in place of
        /// listing every capability trait: a module's
        /// `handlers<P: Capabilities>() -> Handlers<P>`, or the root router
        /// fn a transport `export!` binds. A handler still bounds its own `P`
        /// on the capabilities it reaches. [`WasiProvider`] satisfies it on
        /// `wasm32`, as does the `omnia_test::guest` provider natively.
        /// `DocumentStore` and `TableStore` are included under the `docstore`
        /// and `sql` features.
        ///
        /// Blanket-implemented for every such provider; never implemented by
        /// hand.
        pub trait Capabilities:
            BlobStore
            + Broadcast
            + Config
            + HttpRequest
            + Identity
            + Model
            + Plugins
            + Publish
            + StateStore
            + Vault
            + Vcs
            $(+ $gated)*
            + Send
            + Sync
            + 'static
        {
        }

        impl<P> Capabilities for P where
            P: BlobStore
                + Broadcast
                + Config
                + HttpRequest
                + Identity
                + Model
                + Plugins
                + Publish
                + StateStore
                + Vault
                + Vcs
                $(+ $gated)*
                + Send
                + Sync
                + 'static
        {
        }
    };
}

cfg_select! {
    all(feature = "docstore", feature = "sql") => {
        capabilities!(DocumentStore, TableStore);
    }
    feature = "docstore" => {
        capabilities!(DocumentStore);
    }
    feature = "sql" => {
        capabilities!(TableStore);
    }
    _ => {
        capabilities!();
    }
}
