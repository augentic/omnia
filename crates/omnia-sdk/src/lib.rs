#![doc = include_str!("../README.md")]

pub mod api;
mod capabilities;
mod error;
#[cfg(feature = "http")]
pub mod mcp;

/// Blobstore metadata value types.
pub mod blobstore {
    pub use omnia_backend::blobstore::{ContainerMetadata, ObjectMetadata};
}

/// Document store value types and filter builders.
#[cfg(feature = "docstore")]
pub mod docstore {
    pub use omnia_backend::docstore::{
        ComparisonOp, Document, Filter, QueryOptions, QueryResult, ScalarValue, SortField,
        Timestamp,
    };
}

/// Identity access-token value type.
pub mod identity {
    pub use omnia_backend::identity::AccessToken;
}

/// Key-value wire constants.
pub mod keyvalue {
    pub use omnia_backend::keyvalue::BUCKET;
}

/// SQL table value types.
#[cfg(feature = "sql")]
pub mod sql {
    pub use omnia_backend::sql::{DataType, Field, Row};
}

/// Secret vault wire constants.
pub mod vault {
    pub use omnia_backend::vault::LOCKER;
}

#[cfg(feature = "http")]
#[doc(hidden)]
pub use axum;
pub use omnia_guest_macros::Provider;
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

#[cfg(feature = "http")]
pub use crate::api::http::{HttpError, HttpResult};
pub use crate::capabilities::*;
pub use crate::error::*;
