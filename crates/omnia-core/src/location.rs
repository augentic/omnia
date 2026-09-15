//! Plugin acquisition locations carried on a deployment.

use std::path::PathBuf;

use serde::Deserialize;

/// One place the plugin loader acquires packages from, discriminated by the
/// keys present: `{ name, path }` or `{ registry, config? }`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum Location {
    /// A named host directory path loads resolve against.
    Path {
        /// The location name a load's `path` location names (e.g. `.`).
        name: String,
        /// Host directory. Relative paths resolve against the config file's directory.
        path: PathBuf,
    },
    /// The deployment's registry policy: a default endpoint and, optionally,
    /// a wasm-pkg client configuration routing namespaces and packages to
    /// other registries.
    Registry {
        /// The registry a package resolves against when nothing routes it
        /// elsewhere.
        registry: String,
        /// wasm-pkg client configuration, as TOML: `namespace_registries`,
        /// `package_registry_overrides`, and per-registry backend settings.
        /// The reachable registries are fixed by the deployment, so the
        /// configuration is compiled in rather than read from a user file.
        #[serde(default)]
        config: Option<String>,
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
            config: None,
        }
    }

    /// The default registry endpoint with wasm-pkg client configuration
    /// routing namespaces and packages to other registries.
    pub fn registry_config(registry: impl Into<String>, config: impl Into<String>) -> Self {
        Self::Registry {
            registry: registry.into(),
            config: Some(config.into()),
        }
    }
}
