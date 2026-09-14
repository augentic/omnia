//! Installing the loader over a deployment's declared locations — the one
//! place the built-in acquirers are named concretely.

use std::sync::Arc;

use anyhow::{Context as _, bail};
use omnia_core::{Location, Runtime};
use wasm_pkg_client::Config;

use crate::loader::Plugins;
use crate::path::PathMounts;
use crate::registry::RegistryClient;
use crate::source::{PathSource, RegistrySource};

impl Plugins {
    /// Install the loader capability over the deployment's declared
    /// locations ([`Runtime::plugin_locations`]): every path entry folds, in
    /// declaration order, into one [`PathMounts`] filling the path slot, the
    /// registry entry into a cacheless [`RegistryClient`] — routing through
    /// the entry's wasm-pkg configuration when it carries one — filling the
    /// registry slot. A deployment declaring no locations installs nothing,
    /// so a load refuses as loader misconfiguration.
    ///
    /// # Errors
    ///
    /// Returns an error if a path location cannot be opened, a registry
    /// configuration does not parse or names another default registry, or
    /// the capability is already installed.
    pub fn install_declared<B>(runtime: &Runtime<B>) -> anyhow::Result<()>
    where
        B: Clone + Send + Sync + 'static,
    {
        let locations = runtime.plugin_locations();
        if locations.is_empty() {
            return Ok(());
        }
        let paths: Vec<(&str, &std::path::Path)> = locations
            .iter()
            .filter_map(|location| match location {
                Location::Path { name, path } => Some((name.as_str(), path.as_path())),
                Location::Registry { .. } => None,
            })
            .collect();
        let path: Option<Arc<dyn PathSource>> =
            if paths.is_empty() { None } else { Some(Arc::new(PathMounts::new(paths)?)) };
        let registry = locations
            .iter()
            .find_map(|location| match location {
                Location::Registry { registry, config } => {
                    Some(registry_client(registry, config.as_deref()))
                }
                Location::Path { .. } => None,
            })
            .transpose()?
            .map(|client| Arc::new(client) as Arc<dyn RegistrySource>);
        Self::install(runtime, registry, path)
    }
}

/// A cacheless acquirer defaulting to `registry`, routing through the
/// wasm-pkg `config` (TOML) when the location carries one.
fn registry_client(registry: &str, config: Option<&str>) -> anyhow::Result<RegistryClient> {
    let client = RegistryClient::new(registry);
    let Some(config) = config else {
        return Ok(client);
    };
    let config = Config::from_toml(config).with_context(|| {
        format!("parsing the wasm-pkg configuration of registry location `{registry}`")
    })?;
    // The location's endpoint is the default. A configuration naming another
    // would route every unmapped package past the declared one silently.
    if let Some(declared) = config.default_registry()
        && declared.to_string() != registry
    {
        bail!(
            "registry location `{registry}` carries a configuration whose `default_registry` is \
             `{declared}`; drop the key or make them agree"
        );
    }
    Ok(client.with_config(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The configuration is optional; without it the endpoint alone is the policy.
    #[test]
    fn bare_registry() {
        registry_client("ghcr.io", None).expect("a bare endpoint installs");
    }

    // Routing tables and an agreeing `default_registry` are accepted.
    #[test]
    fn routed_registry() {
        let config =
            "default_registry = \"ghcr.io\"\n\n[namespace_registries]\nwasi = \"wasi.dev\"\n";
        registry_client("ghcr.io", Some(config)).expect("routing beside the endpoint installs");
    }

    // A configuration naming another default would silently route past the
    // declared endpoint.
    #[test]
    fn default_conflict() {
        let config = "default_registry = \"docker.io\"\n";
        let error = registry_client("ghcr.io", Some(config)).err().expect("two defaults refuse");
        assert!(error.to_string().contains("`default_registry` is `docker.io`"), "{error}");
    }

    // Malformed TOML fails the install, not the first load.
    #[test]
    fn malformed_config() {
        let error =
            registry_client("ghcr.io", Some("[namespace_registries")).err().expect("refused");
        assert!(error.to_string().contains("registry location `ghcr.io`"), "{error}");
    }
}
