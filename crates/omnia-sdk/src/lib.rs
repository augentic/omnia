#![doc = include_str!("../README.md")]

pub mod api;
mod capabilities;
#[cfg(feature = "docstore")]
pub mod docstore;
mod error;
#[cfg(feature = "http")]
pub mod mcp;
#[cfg(feature = "sql")]
pub mod sql;

pub use crate::capabilities::*;
pub use crate::error::*;

/// Blobstore metadata value types.
pub mod blobstore {
    pub use crate::{ContainerMetadata, ObjectMetadata};
}

/// Identity access-token value type.
pub mod identity {
    pub use crate::AccessToken;
}

/// Key-value wire constants.
pub mod keyvalue {
    /// The bucket [`StateStore`](crate::StateStore) opens.
    ///
    /// A value written without a TTL stays until deleted, on every backend.
    pub const BUCKET: &str = "state";
}

/// Secret vault wire constants.
pub mod vault {
    /// The locker the [`Vault`](crate::Vault) capability opens.
    pub const LOCKER: &str = "locker";
}

#[cfg(feature = "http")]
#[doc(hidden)]
pub use axum;
pub use omnia_guest_macros::{Provider, Settings};
#[doc(hidden)]
pub mod http {
    pub use ::http::*;
}
#[doc(hidden)]
pub use {anyhow, bytes, http_body, tracing};
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
pub use {
    omnia_wasi_blobstore, omnia_wasi_http, omnia_wasi_identity, omnia_wasi_keyvalue,
    omnia_wasi_messaging, omnia_wasi_otel, omnia_wasi_vault, omnia_wasi_vcs, wasip3, wit_bindgen,
};
