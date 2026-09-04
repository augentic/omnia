//! Plugin acquisition locations carried on a deployment.

use std::path::PathBuf;

use serde::Deserialize;

/// One place the plugin loader acquires packages from, discriminated by the
/// keys present: `{ name, path }` or `{ registry }`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum Location {
    /// A named host directory path loads resolve against.
    Path {
        /// The location name a load's `path` location names (e.g. `.`).
        name: String,
        /// Host directory. [`crate::Manifest::from_config`] resolves relative paths
        /// against the config file's directory.
        path: PathBuf,
    },
    /// The deployment's default registry endpoint.
    Registry {
        /// The registry a load without an explicit endpoint resolves against.
        registry: String,
    },
}

impl Location {
    /// A named path root.
    pub fn path(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::Path {
            name: name.into(),
            path: path.into(),
        }
    }

    /// The default registry endpoint.
    pub fn registry(registry: impl Into<String>) -> Self {
        Self::Registry {
            registry: registry.into(),
        }
    }
}
