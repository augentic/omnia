//! Binary large object storage service contract, value types, and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "blobstore-default"))]
mod default;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{BlobStoreClient, Container};
#[cfg(all(not(target_arch = "wasm32"), feature = "blobstore-default"))]
pub use self::default::BlobStoreDefault;

/// Metadata for a blobstore container.
///
/// Mirrors the `container-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMetadata {
    /// The container's name.
    pub name: String,
    /// Seconds since Unix epoch when the container was created.
    pub created_at: u64,
}

/// Metadata for an object in a blobstore container.
///
/// Mirrors the `object-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    /// The object's name.
    pub name: String,
    /// The object's parent container.
    pub container: String,
    /// Seconds since Unix epoch when the object was created.
    pub created_at: u64,
    /// Size of the object in bytes.
    pub size: u64,
}
