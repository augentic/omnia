use std::collections::HashMap;

pub use omnia_backend::messaging::{Endpoint, Error, Message, Metadata, RequestOptions};

use crate::host::generated::wasi::messaging::types;

/// Proxy for a messaging endpoint.
pub type ClientProxy = omnia_core::Proxy<dyn Endpoint>;

pub fn to_wit_metadata(meta: Metadata) -> types::Metadata {
    let mut metadata = types::Metadata::new();
    for (k, v) in meta.inner {
        metadata.push((k, v));
    }
    metadata
}

pub fn from_wit_metadata(meta: types::Metadata) -> Metadata {
    let mut map = HashMap::new();
    for (k, v) in meta {
        map.insert(k, v);
    }
    Metadata { inner: map }
}

pub fn from_wit_error(err: types::Error) -> Error {
    match err {
        types::Error::Timeout => Error::Timeout,
        types::Error::Connection(s) => Error::Connection(s),
        types::Error::PermissionDenied(s) => Error::PermissionDenied(s),
        types::Error::Other(s) => Error::Other(s),
    }
}
