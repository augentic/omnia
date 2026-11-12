//! Blob store value types.

use std::time::SystemTime;

/// Metadata for a blob store container.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContainerMetadata {
    /// The container's name.
    pub name: String,

    /// When the container was created.
    pub created_at: SystemTime,
}

impl ContainerMetadata {
    /// Metadata for the container `name`, created at `created_at`.
    pub fn new(name: impl Into<String>, created_at: SystemTime) -> Self {
        Self {
            name: name.into(),
            created_at,
        }
    }
}

/// Metadata for an object in a blob store container.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObjectMetadata {
    /// The object's name.
    pub name: String,

    /// The container holding the object.
    pub container: String,

    /// When the object was written.
    pub created_at: SystemTime,

    /// The object's size in bytes.
    pub size: u64,
}

impl ObjectMetadata {
    /// Metadata for the object `name` in `container`, written at `created_at`.
    pub fn new(
        name: impl Into<String>, container: impl Into<String>, created_at: SystemTime, size: u64,
    ) -> Self {
        Self {
            name: name.into(),
            container: container.into(),
            created_at,
            size,
        }
    }
}
