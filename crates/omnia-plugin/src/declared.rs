//! Installing the loader over a deployment's declared policy — its mounts and
//! its `registries` configuration — the one place the built-in acquirers are
//! named concretely.

use std::sync::Arc;

use anyhow::Context as _;
use omnia_core::Runtime;
use wasm_pkg_client::Config;

use crate::loader::Plugins;
use crate::path::PathMounts;
use crate::registry::RegistryClient;
use crate::source::PathSource;

impl Plugins {
    /// Install the loader capability over the deployment's declared policy:
    /// its mounts ([`Runtime::mounts`]) become the roots path loads resolve
    /// against, borrowing the directories already open for the guest
    /// sandbox, and its `registries` configuration
    /// ([`Runtime::registry_config`]) becomes the default routing of a
    /// cacheless [`RegistryClient`]. A deployment declaring no mounts refuses
    /// every path load typed; one declaring no `registries` still serves a
    /// package load that names its registry, and refuses one that does not.
    ///
    /// # Errors
    ///
    /// Returns an error if the `registries` configuration does not parse, or
    /// the capability is already installed.
    pub fn install_declared<B>(runtime: &Runtime<B>) -> anyhow::Result<()>
    where
        B: Clone + Send + Sync + 'static,
    {
        let mounts = runtime.mounts();
        let path = (!mounts.entries().is_empty())
            .then(|| Arc::new(PathMounts::from(mounts)) as Arc<dyn PathSource>);
        let registry = RegistryClient::new(routing(runtime.registry_config())?);
        Self::install(runtime, Some(Arc::new(registry)), path)
    }
}

/// The wasm-pkg `config` a deployment's `registries` TOML parses to; empty
/// routing when it declares none.
fn routing(config: Option<&str>) -> anyhow::Result<Config> {
    config.map_or_else(
        || Ok(Config::empty()),
        |config| {
            Config::from_toml(config).context("parsing the deployment's `registries` configuration")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Routing tables and a `default_registry` install as given.
    #[test]
    fn routed_registry() {
        let config =
            "default_registry = \"ghcr.io\"\n\n[namespace_registries]\nwasi = \"wasi.dev\"\n";
        routing(Some(config)).expect("a routed configuration installs");
    }

    // A configuration naming no default still installs: an unrouted package
    // is refused at load, naming its namespace.
    #[test]
    fn no_default_registry() {
        routing(Some("[namespace_registries]\nwasi = \"wasi.dev\"\n"))
            .expect("a defaultless configuration installs");
    }

    // No configuration installs too: a load naming its registry still
    // resolves, and one naming none is refused at load.
    #[test]
    fn no_config() {
        let config = routing(None).expect("no configuration installs");
        let package = "acme:tool".parse().expect("a package reference");
        assert!(config.resolve_registry(&package).is_none());
    }

    // Malformed TOML fails the install, not the first load.
    #[test]
    fn malformed_config() {
        let error = routing(Some("[namespace_registries")).expect_err("refused");
        assert!(error.to_string().contains("`registries` configuration"), "{error}");
    }
}
