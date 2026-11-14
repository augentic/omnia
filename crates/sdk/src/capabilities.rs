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
//! Every capability has a companion `Has<Capability>` accessor trait (emitted
//! by `#[capability]`) with a blanket forwarding impl and implementations for
//! `Arc<P>`, `&P`, and `Box<P>`, so `#[derive(Provider)]` and pointer
//! wrappers delegate without repeating method signatures.
//!
//! A `<Capability>Ext` trait ([`StateStoreExt`], [`ConfigExt`], ...) holds the
//! caller-side codecs and parsers over a capability's primitives. It is
//! blanket-implemented for every implementor and never implemented by hand, so
//! one encoding holds across providers. Its methods return
//! [`capability::Result`](self::capability::Result), with a codec failure as
//! [`ErrorKind::Invalid`](self::capability::ErrorKind::Invalid).
//! [`HttpClientExt::fetch_json`] alone returns [`crate::Result`], since it
//! also maps a non-2xx status to [`Error::BadGateway`](crate::Error::BadGateway).
//!
//! Failed calls and listings use [`capability`](self::capability) below.
//! Absence is not a failure: a missing key is `Ok(None)`.

/// The error, result, and listing page a capability call returns.
pub mod capability {
    use std::fmt;

    /// The result of a capability call.
    pub type Result<T, E = Error> = std::result::Result<T, E>;

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

    /// A failed capability call.
    ///
    /// A caller matches on [`kind`](Self::kind). The message is for people, and
    /// its text is not part of the contract.
    #[derive(Debug, thiserror::Error)]
    #[error("{kind}: {message}")]
    pub struct Error {
        kind: ErrorKind,
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    }

    impl Error {
        /// Create an error of `kind` described by `message`.
        pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
            Self {
                kind,
                message: message.into(),
                source: None,
            }
        }

        /// Attach the error that caused this one.
        #[must_use]
        pub fn with_source(
            mut self, source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
        ) -> Self {
            self.source = Some(source.into());
            self
        }

        /// The kind of failure.
        #[must_use]
        pub const fn kind(&self) -> ErrorKind {
            self.kind
        }
    }

    /// What kind of failure a capability call met.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum ErrorKind {
        /// The binding, container, collection, or required setting does not exist.
        NotFound,

        /// The provider refused the call.
        Denied,

        /// The input was rejected, such as a zero TTL, or stored bytes failed to decode.
        Invalid,

        /// The request is valid, but this provider cannot serve it.
        Unsupported,

        /// The upstream did not answer, or the call exceeded its time bound.
        Unavailable,

        /// Any other failure.
        Other,
    }

    impl fmt::Display for ErrorKind {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                Self::NotFound => "not found",
                Self::Denied => "denied",
                Self::Invalid => "invalid",
                Self::Unsupported => "unsupported",
                Self::Unavailable => "unavailable",
                Self::Other => "other",
            })
        }
    }
}

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