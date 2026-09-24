//! Registry acquisition over [wasm-pkg-client].
//!
//! [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools

use std::io;

use anyhow::{Context as _, Result, bail};
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use omnia_core::Digest;
use wasm_pkg_client::{Client, Config, ContentStream, PackageRef, Registry, Release, Version};

use crate::error::LoadError;
use crate::source::RegistrySource;
use crate::store::{ContentStore, NoStore, ReleaseStore};

/// Registry acquisition using [wasm-pkg-client].
///
/// Fetches exact `namespace:name@version` references only, verifying every
/// result against the registry's content digest. The configuration alone
/// routes a package — its `package_registry_overrides` entry, its
/// namespace's `namespace_registries` entry, or the `default_registry` —
/// and a package it routes nowhere is refused. The attached store is a byte
/// cache and offline fallback — never the authority while the registry is
/// reachable — so a failing store degrades a load, never refuses it.
///
/// [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools
pub struct RegistryClient<S = NoStore> {
    config: Config,
    store: S,
}

impl RegistryClient<NoStore> {
    /// Cacheless acquirer routing every package through `config`.
    ///
    /// The configuration is exactly what the deployment declares — no
    /// user-global wasm-pkg config file and no hard-coded fallback registries
    /// are consulted.
    #[must_use]
    pub const fn new(config: Config) -> Self {
        Self {
            config,
            store: NoStore,
        }
    }

    /// Cacheless acquirer routing through a deployment's `registries` TOML.
    ///
    /// # Errors
    ///
    /// Returns an error if `config` is not a valid wasm-pkg configuration.
    pub fn from_toml(config: &str) -> Result<Self> {
        Config::from_toml(config)
            .context("parsing the deployment's `registries` configuration")
            .map(Self::new)
    }
}

// Routes nothing: the acquirer of a deployment declaring no `registries`,
// which refuses every package load naming its namespace.
impl Default for RegistryClient<NoStore> {
    fn default() -> Self {
        Self::new(Config::empty())
    }
}

impl<S: ContentStore + ReleaseStore> RegistryClient<S> {
    /// Attaches a store as byte cache and offline fallback.
    #[must_use]
    pub fn cached<S2: ContentStore + ReleaseStore>(self, store: S2) -> RegistryClient<S2> {
        RegistryClient {
            config: self.config,
            store,
        }
    }

    fn registry(&self, package: &PackageRef) -> Result<Registry, LoadError> {
        self.config.resolve_registry(package).cloned().ok_or_else(|| {
            LoadError::Refused(format!(
                "no registry routes `{package}`: the deployment's `registries` routes neither the \
                 `{}` namespace nor a default",
                package.namespace()
            ))
        })
    }

    /// Resolve and fetch `package`, serving verified bytes from the store
    /// when possible.
    async fn fetch(&self, package: &str) -> Result<Vec<u8>, LoadError> {
        let (package_ref, version) =
            parse_package(package).map_err(|error| LoadError::Refused(format!("{error:#}")))?;
        let registry = self.registry(&package_ref)?.to_string();
        // Loads are rare, so a fresh client per fetch beats caching machinery.
        let client = Client::new(self.config.clone());
        let release =
            self.resolve_release(&client, &registry, package, &package_ref, &version).await?;
        let expected: Digest = release.content_digest.to_string().parse().map_err(|error| {
            LoadError::Refused(format!(
                "the registry digest for `{package}` is unsupported: {error}"
            ))
        })?;

        if let Some(bytes) = self.stored(package, expected).await {
            return Ok(bytes);
        }

        let content = client
            .stream_content(&package_ref, &release)
            .await
            .map_err(|error| LoadError::Unavailable(format!("fetching `{package}`: {error}")))?;
        let bytes = collect(content)
            .await
            .map_err(|error| LoadError::Unavailable(format!("reading `{package}`: {error}")))?;

        let resolved = Digest::of(&bytes);
        if resolved != expected {
            // The registry misdelivered; a retry may serve honest bytes.
            return Err(LoadError::Unavailable(format!(
                "package `{package}` content hashes to {resolved}, not the registry digest \
                 {expected}"
            )));
        }
        if let Err(error) = self.store.put_content(&expected.to_string(), &bytes).await {
            tracing::warn!(
                package,
                %expected,
                error = format!("{error:#}"),
                "failed to store the package content"
            );
        }
        tracing::debug!(package, digest = %resolved, "package acquired");
        Ok(bytes)
    }

    /// The store's verified bytes for `digest`; `None` on a miss, a failed
    /// verification, or an unreadable store — the cache never refuses a load.
    async fn stored(&self, package: &str, digest: Digest) -> Option<Vec<u8>> {
        match self.store.content(&digest.to_string()).await {
            Ok(Some(bytes)) => {
                // A poisoned entry must never become code; discard and refetch.
                if Digest::of(&bytes) == digest {
                    tracing::debug!(package, %digest, "package served from the store");
                    Some(bytes)
                } else {
                    tracing::warn!(
                        package,
                        %digest,
                        "stored content failed verification; discarding and refetching"
                    );
                    None
                }
            }
            Ok(None) => None,
            Err(error) => {
                // Cache, never authority: an unreadable store degrades to a
                // fresh fetch.
                tracing::warn!(
                    package,
                    %digest,
                    error = format!("{error:#}"),
                    "failed to read the store; fetching fresh"
                );
                None
            }
        }
    }

    /// Resolve the release fresh, refreshing the store's record; fall back
    /// to the stored record — logged — only on a network failure.
    async fn resolve_release(
        &self, client: &Client, registry: &str, package: &str, package_ref: &PackageRef,
        version: &Version,
    ) -> Result<Release, LoadError> {
        let full_name = package_ref.to_string();
        match client.get_release(package_ref, version).await {
            Ok(release) => {
                let digest = release.content_digest.to_string();
                if let Err(error) = self
                    .store
                    .put_release(registry, &full_name, &version.to_string(), &digest)
                    .await
                {
                    tracing::warn!(
                        package,
                        registry,
                        error = format!("{error:#}"),
                        "failed to record the release"
                    );
                }
                Ok(release)
            }
            Err(error) if is_network_failure(&error) => {
                let stored = self
                    .store
                    .release(registry, &full_name, &version.to_string())
                    .await
                    .map_err(|error| LoadError::Unavailable(format!("{error:#}")))?;
                let Some(digest) = stored else {
                    return Err(LoadError::Unavailable(format!("resolving `{package}`: {error}")));
                };
                tracing::warn!(
                    package,
                    registry,
                    error = format!("{error:#}"),
                    "registry unreachable; falling back to the stored release record"
                );
                let content_digest = digest.parse().map_err(|error| {
                    LoadError::Unavailable(format!(
                        "stored release record for `{package}` carries a malformed digest: {error}"
                    ))
                })?;
                Ok(Release {
                    version: version.clone(),
                    content_digest,
                })
            }
            // An authoritative registry answer — not found, yanked, malformed
            // input — refuses: retrying the same reference cannot succeed.
            Err(error) => Err(LoadError::Refused(format!("resolving `{package}`: {error}"))),
        }
    }
}

impl<S: ContentStore + ReleaseStore> RegistrySource for RegistryClient<S> {
    fn acquire<'a>(&'a self, package: &'a str) -> BoxFuture<'a, Result<Vec<u8>, LoadError>> {
        self.fetch(package).boxed()
    }
}

/// Whether a resolution error is a transport failure — endpoint unreachable,
/// registry misbehaving — rather than an authoritative registry answer
/// (not found, yanked, malformed input), which must never be papered over
/// by a stored record.
fn is_network_failure(error: &wasm_pkg_client::Error) -> bool {
    match error {
        wasm_pkg_client::Error::RegistryError(source)
        | wasm_pkg_client::Error::RegistryMetadataError(source) => !is_not_found(source),
        wasm_pkg_client::Error::IoError(source) => source.kind() != io::ErrorKind::NotFound,
        _ => false,
    }
}

// A backend that serves releases from storage (wasm-pkg-client's `local`)
// reports a version it lacks as an I/O `NotFound` inside a registry error;
// that is the registry's answer, not a fault on the way to it.
fn is_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|io| io.kind() == io::ErrorKind::NotFound)
    })
}

/// Drain `stream` into memory; callers hash the whole buffer anyway.
async fn collect(mut stream: ContentStream) -> Result<Vec<u8>, wasm_pkg_client::Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.try_next().await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Split an exact `namespace:name@version` reference; remote lookup never
/// resolves "latest".
fn parse_package(package: &str) -> Result<(PackageRef, Version)> {
    let Some((name, version)) = package.split_once('@') else {
        bail!("registry package `{package}` must pin an exact version (`namespace:name@version`)")
    };
    let package_ref = name.parse().with_context(|| {
        format!("package `{package}` is not a `namespace:name@version` reference")
    })?;
    let version = version
        .parse()
        .with_context(|| format!("package `{package}` does not pin an exact semver version"))?;
    Ok((package_ref, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(reference: &str) -> PackageRef {
        reference.parse().expect("a package reference")
    }

    #[test]
    fn routed_registry() {
        let client = RegistryClient::from_toml(
            "default_registry = \"ghcr.io\"\n\n[namespace_registries]\nwasi = \"wasi.dev\"\n",
        )
        .expect("a routed configuration parses");
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
        let client = RegistryClient::from_toml("[namespace_registries]\nwasi = \"wasi.dev\"\n")
            .expect("a defaultless configuration parses");
        assert!(client.config.resolve_registry(&package("acme:tool")).is_none());
    }

    #[test]
    fn default_routes_nothing() {
        let client = RegistryClient::default();
        assert!(client.config.resolve_registry(&package("acme:tool")).is_none());
        let error = client.registry(&package("acme:tool")).expect_err("refused");
        assert!(matches!(error, LoadError::Refused(detail) if detail.contains("`acme` namespace")));
    }

    // Malformed TOML fails the install, not the first load.
    #[test]
    fn malformed_config() {
        let Err(error) = RegistryClient::from_toml("[namespace_registries") else {
            panic!("malformed TOML must be refused");
        };
        assert!(error.to_string().contains("`registries` configuration"), "{error}");
    }

    #[test]
    fn package_reference() {
        let (package_ref, version) = parse_package("acme:tool@1.2.3").expect("exact reference");
        assert_eq!(package_ref.to_string(), "acme:tool");
        assert_eq!(version.to_string(), "1.2.3");
        for malformed in ["acme:tool", "acme:tool@latest", "tool@1.2.3"] {
            parse_package(malformed).expect_err("refused");
        }
    }
}
