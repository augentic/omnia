//! Registry acquisition over [wasm-pkg-client], store first.
//!
//! [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools

use std::io;

use anyhow::{Context as _, Result};
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use omnia_core::{AcquireError, Digest};
use wasm_pkg_client::{Client, Config, ContentStream, PackageRef, Registry, RegistryMapping};
use wasmtime::Engine;

use crate::source::RegistrySource;
use crate::store::{NoStore, PackageStore, Reference};

/// Registry acquisition using [wasm-pkg-client], through a [`PackageStore`].
///
/// Fetches exact `namespace:name@version` references only. The store answers
/// first: a release it holds is served with no network, whoever wrote it. One
/// it lacks is fetched, verified against the registry's content digest, and
/// written to the store once — a stored release is final until removed. The
/// configuration bounds where a package is fetched from: one it routes — by
/// its `package_registry_overrides` entry, its namespace's
/// `namespace_registries` entry, or the `default_registry` — is fetched from
/// that registry alone, and a load naming another is refused; one it routes
/// nowhere is fetched from the registry the load names, and refused, naming
/// the namespace and the store, when it names none.
///
/// [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools
pub struct RegistryClient<S = NoStore> {
    config: Config,
    store: S,
}

impl<S: PackageStore> RegistryClient<S> {
    /// An acquirer routing every package through `config` and keeping what
    /// it fetches in `store`.
    ///
    /// The configuration is exactly what the deployment declares — no
    /// user-global wasm-pkg config file and no hard-coded fallback registries
    /// are consulted.
    #[must_use]
    pub const fn new(config: Config, store: S) -> Self {
        Self { config, store }
    }

    /// An acquirer routing through a deployment's `registries` TOML; `None`
    /// routes nothing, so a release the store lacks is fetched from the
    /// registry the load names alone.
    ///
    /// # Errors
    ///
    /// Returns an error if `config` is not a valid wasm-pkg configuration.
    pub fn from_toml(config: Option<&str>, store: S) -> Result<Self> {
        let config = match config {
            Some(config) => Config::from_toml(config)
                .context("parsing the deployment's `registries` configuration")?,
            None => Config::empty(),
        };
        Ok(Self::new(config, store))
    }

    // A routed package is served by its registry alone; one routed nowhere by
    // the registry the load names, and refused when it names none.
    fn registry(
        &self, reference: &Reference, endpoint: Option<&str>,
    ) -> Result<Registry, AcquireError> {
        let package = reference.package();
        let named = endpoint
            .map(|endpoint| {
                endpoint.parse::<Registry>().map_err(|error| {
                    AcquireError::Refused(format!(
                        "registry `{endpoint}` is not a valid name: {error}"
                    ))
                })
            })
            .transpose()?;
        match (self.config.resolve_registry(package), named) {
            (Some(routed), Some(named)) if *routed != named => Err(AcquireError::Refused(format!(
                "`{package}` is routed to `{routed}` by the deployment's `registries`; it cannot \
                 be fetched from `{named}`"
            ))),
            (Some(routed), _) => Ok(routed.clone()),
            (None, Some(named)) => Ok(named),
            (None, None) => Err(AcquireError::Refused(format!(
                "no registry routes `{reference}`: the load names none, the deployment's \
                 `registries` routes neither the `{}` namespace nor a default, and {}",
                package.namespace(),
                self.store.describe(reference)
            ))),
        }
    }

    // A fresh client per fetch, since loads are rare; a package routed nowhere
    // is routed to `registry` for this fetch alone.
    fn client(&self, package: &PackageRef, registry: &Registry) -> Client {
        let mut config = self.config.clone();
        if config.resolve_registry(package).is_none() {
            config.set_package_registry_override(
                package.clone(),
                RegistryMapping::Registry(registry.clone()),
            );
        }
        Client::new(config)
    }

    async fn acquire(
        &self, package: &str, endpoint: Option<&str>,
    ) -> Result<Vec<u8>, AcquireError> {
        let reference: Reference = package
            .parse()
            .map_err(|error: anyhow::Error| AcquireError::Refused(format!("{error:#}")))?;

        // the store answers first, with no network
        let stored = self
            .store
            .get(&reference)
            .await
            .map_err(|error| AcquireError::Unavailable(format!("{error:#}")))?;
        if let Some(bytes) = stored {
            tracing::info!(package, digest = %Digest::of(&bytes), "package served from the store");
            return Ok(bytes);
        }

        // then the registry the deployment routes it to
        let registry = self.registry(&reference, endpoint)?;
        let bytes = self.fetch(&reference, &registry).await?;
        if let Err(error) = self.store.put(&reference, &bytes).await {
            tracing::warn!(package, error = format!("{error:#}"), "failed to store the package");
        }
        tracing::info!(
            package,
            registry = %registry,
            digest = %Digest::of(&bytes),
            "package fetched from the registry"
        );
        Ok(bytes)
    }

    // Resolve the release, stream its content, and hold the bytes to the
    // registry's digest; raw wasm alone leaves here, so nothing pre-compiled
    // is ever stored.
    async fn fetch(
        &self, reference: &Reference, registry: &Registry,
    ) -> Result<Vec<u8>, AcquireError> {
        let package = reference.package();
        let client = self.client(package, registry);
        let release = client.get_release(package, reference.version()).await.map_err(|error| {
            if is_network_failure(&error) {
                AcquireError::Unavailable(format!("resolving `{reference}`: {error}"))
            } else {
                // an authoritative answer refuses: retrying the same reference cannot succeed
                AcquireError::Refused(format!("resolving `{reference}`: {error}"))
            }
        })?;
        let expected: Digest = release.content_digest.to_string().parse().map_err(|error| {
            AcquireError::Refused(format!(
                "the registry digest for `{reference}` is unsupported: {error}"
            ))
        })?;

        let content = client.stream_content(package, &release).await.map_err(|error| {
            AcquireError::Unavailable(format!("fetching `{reference}`: {error}"))
        })?;
        let bytes = collect(content).await.map_err(|error| {
            AcquireError::Unavailable(format!("reading `{reference}`: {error}"))
        })?;

        let resolved = Digest::of(&bytes);
        if resolved != expected {
            // the registry misdelivered; a retry may serve honest bytes
            return Err(AcquireError::Unavailable(format!(
                "package `{reference}` content hashes to {resolved}, not the registry digest \
                 {expected}"
            )));
        }
        if Engine::detect_precompiled(&bytes).is_some() {
            return Err(AcquireError::Refused(format!(
                "package `{reference}` is pre-compiled, but a package admits raw wasm alone"
            )));
        }
        Ok(bytes)
    }
}

// Routes nothing and stores nothing: the acquirer of a deployment declaring
// no `plugins`, which refuses every package load naming its namespace.
impl Default for RegistryClient<NoStore> {
    fn default() -> Self {
        Self::new(Config::empty(), NoStore)
    }
}

impl<S: PackageStore> RegistrySource for RegistryClient<S> {
    fn acquire<'a>(
        &'a self, package: &'a str, endpoint: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        self.acquire(package, endpoint).boxed()
    }
}

// A transport failure, as opposed to an authoritative answer (not found,
// yanked, malformed input).
fn is_network_failure(error: &wasm_pkg_client::Error) -> bool {
    match error {
        wasm_pkg_client::Error::RegistryError(source)
        | wasm_pkg_client::Error::RegistryMetadataError(source) => !is_not_found(source),
        wasm_pkg_client::Error::IoError(source) => source.kind() != io::ErrorKind::NotFound,
        _ => false,
    }
}

// wasm-pkg-client's `local` backend reports a version it lacks as an I/O
// `NotFound` inside a registry error: the registry's answer, not a fault
fn is_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|io| io.kind() == io::ErrorKind::NotFound)
    })
}

async fn collect(mut stream: ContentStream) -> Result<Vec<u8>, wasm_pkg_client::Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.try_next().await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(reference: &str) -> PackageRef {
        reference.parse().expect("a package reference")
    }

    fn reference(spelled: &str) -> Reference {
        spelled.parse().expect("an exact reference")
    }

    fn routed(toml: &str) -> RegistryClient {
        RegistryClient::from_toml(Some(toml), NoStore).expect("a configuration parses")
    }

    #[test]
    fn routed_registry() {
        let client = routed(
            "default_registry = \"ghcr.io\"\n\n[namespace_registries]\nwasi = \"wasi.dev\"\n",
        );
        assert_eq!(
            client.config.resolve_registry(&package("wasi:http")).map(ToString::to_string),
            Some("wasi.dev".to_owned())
        );
        assert_eq!(
            client.config.resolve_registry(&package("acme:tool")).map(ToString::to_string),
            Some("ghcr.io".to_owned())
        );
    }

    // A configuration naming no default still parses: an unrouted package is
    // refused at load, naming its namespace.
    #[test]
    fn no_default_registry() {
        let client = routed("[namespace_registries]\nwasi = \"wasi.dev\"\n");
        assert!(client.config.resolve_registry(&package("acme:tool")).is_none());
    }

    #[test]
    fn default_routes_nothing() {
        let client = RegistryClient::default();
        assert!(client.config.resolve_registry(&package("acme:tool")).is_none());
        let error = client.registry(&reference("acme:tool@1.0.0"), None).expect_err("refused");
        assert!(
            matches!(&error, AcquireError::Refused(detail) if detail.contains("`acme` namespace") && detail.contains("no store is attached")),
            "{error}"
        );
    }

    // The configuration bounds the load: an unrouted package takes the
    // registry the load names, a routed one is served by its route alone.
    #[test]
    fn endpoint_precedence() {
        let unrouted = RegistryClient::default();
        let tool = reference("acme:tool@1.0.0");
        let named = unrouted.registry(&tool, Some("ghcr.io")).expect("named");
        assert_eq!(named.to_string(), "ghcr.io");
        let error = unrouted.registry(&tool, Some("not a registry")).expect_err("");
        assert!(
            matches!(error, AcquireError::Refused(detail) if detail.contains("not a valid name"))
        );

        let routed = routed("[namespace_registries]\nacme = \"acme.test\"\n");
        let same = routed.registry(&tool, Some("acme.test")).expect("same route");
        assert_eq!(same.to_string(), "acme.test");
        let error = routed.registry(&tool, Some("ghcr.io")).expect_err("refused");
        assert!(
            matches!(&error, AcquireError::Refused(detail) if detail.contains("routed to `acme.test`")),
            "{error}"
        );
    }

    // a routed package keeps its mapping and metadata; only one routed
    // nowhere goes to the registry the load named
    #[test]
    fn client_keeps_routing() {
        let client = routed(
            "[package_registry_overrides]\n\"acme:tool\" = { registry = \"acme.test\", metadata = \
             { preferredProtocol = \"oci\" } }\n",
        );

        let tool = reference("acme:tool@1.0.0");
        let registry = client.registry(&tool, None).expect("routed");
        let mapping = client
            .client(tool.package(), &registry)
            .config()
            .package_registry_override(tool.package())
            .cloned();
        assert!(matches!(mapping, Some(RegistryMapping::Custom(_))), "{mapping:?}");

        let other = reference("acme:other@1.0.0");
        let named = client.registry(&other, Some("ghcr.io")).expect("named");
        let mapping = client
            .client(other.package(), &named)
            .config()
            .package_registry_override(other.package())
            .cloned();
        assert!(
            matches!(&mapping, Some(RegistryMapping::Registry(registry)) if *registry == named),
            "{mapping:?}"
        );
        assert!(client.config.package_registry_override(other.package()).is_none());
    }

    // Malformed TOML fails the install, not the first load.
    #[test]
    fn malformed_config() {
        let Err(error) = RegistryClient::from_toml(Some("[namespace_registries"), NoStore) else {
            panic!("malformed TOML must be refused");
        };
        assert!(error.to_string().contains("`registries` configuration"), "{error}");
    }
}
