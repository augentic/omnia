//! Provider capabilities: the host services a guest's provider can call.
//!
//! Each capability is a target-neutral trait implemented by `WasiProvider` on
//! `wasm32` (delegating to the matching `omnia-wasi-*` binding), by
//! `omnia_test::guest` doubles in native handler tests, and by capability
//! decorators composed via `#[derive(Provider)]`.
//!
//! A capability's module holds its trait and the value types its methods
//! exchange. The traits are re-exported at the crate root, and the value
//! types stay scoped to their module, where a generic name such as `Message`
//! or `Error` is unambiguous.
//!
//! A capability call fails with an [`Error`](crate::Error) whose code names
//! the class of failure. Absence is not a failure: a missing key is
//! `Ok(None)`.
//!
//! Every capability has a companion `Has<Capability>` accessor trait (emitted
//! by `#[capability]`) with a blanket forwarding impl and implementations for
//! `Arc<P>`, `&P`, and `Box<P>`, so `#[derive(Provider)]` and pointer
//! wrappers delegate without repeating method signatures.
//!
//! A `<Capability>Ext` trait ([`StateStoreExt`], [`ConfigExt`], ...) holds the
//! caller-side codecs and parsers over a capability's primitives. It is
//! blanket-implemented for every implementor and never implemented by hand, so
//! one encoding holds across providers. A codec failure has the code
//! `invalid`, except in [`HttpClientExt::fetch_json`], where an upstream's
//! non-2xx status or unparseable body is an
//! [`Error::BadGateway`](crate::Error::BadGateway).

pub mod blobstore;
mod config;
pub mod docstore;
mod http;
pub mod identity;
pub mod keyvalue;
pub mod messaging;
pub mod model;
pub mod plugins;
pub mod sql;
pub mod vault;
pub mod vcs;
mod websockets;

pub use blobstore::BlobStore;
#[doc(hidden)]
pub use blobstore::HasBlobStore;
#[doc(hidden)]
pub use config::HasConfig;
pub use config::{Config, ConfigExt};
#[doc(hidden)]
pub use docstore::HasDocumentDb;
pub use docstore::{DocumentDb, DocumentDbExt};
#[doc(hidden)]
pub use http::HasHttpClient;
pub use http::{HttpClient, HttpClientExt};
#[doc(hidden)]
pub use identity::HasIdentity;
pub use identity::{Identity, IdentityExt};
#[doc(hidden)]
pub use keyvalue::HasStateStore;
pub use keyvalue::{StateStore, StateStoreExt};
#[doc(hidden)]
pub use messaging::HasPublisher;
pub use messaging::Publisher;
#[doc(hidden)]
pub use model::HasModel;
pub use model::Model;
#[doc(hidden)]
pub use plugins::HasPlugins;
pub use plugins::Plugins;
#[doc(hidden)]
pub use sql::HasSqlDb;
pub use sql::{SqlDb, SqlDbExt};
#[doc(hidden)]
pub use vault::HasSecretStore;
pub use vault::SecretStore;
#[doc(hidden)]
pub use vcs::HasVcs;
pub use vcs::Vcs;
#[doc(hidden)]
pub use websockets::HasWebSockets;
pub use websockets::WebSockets;

/// One page of a listing, with the cursor that resumes it.
///
/// Pass [`next`](Self::next) back as the listing's `after` to read the
/// following page. A page without one is the last. A cursor is opaque: it
/// comes from a page and goes back to the listing that made it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Page<T> {
    /// The items on this page.
    pub items: Vec<T>,

    /// The cursor that resumes the listing, or `None` on the last page.
    pub next: Option<String>,
}

impl<T> Page<T> {
    /// A page holding `items` that resumes at `next`.
    #[must_use]
    pub const fn new(items: Vec<T>, next: Option<String>) -> Self {
        Self { items, next }
    }
}
