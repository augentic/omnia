use omnia_backend::blobstore::Container;

/// Proxy for a blobstore container.
pub type ContainerProxy = omnia_core::Proxy<dyn Container>;
