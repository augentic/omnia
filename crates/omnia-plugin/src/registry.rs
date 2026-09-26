//! Registry acquisition over [wasm-pkg-client].
//!
//! [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools

use std::io;

use anyhow::{Context as _, Result, bail};
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use omnia_core::{AcquireError, Digest};
use wasm_pkg_client::{
    Client, Config, ContentStream, PackageRef, Registry, RegistryMapping, Release, Version,
};

use crate::source::RegistrySource;
use crate::store::{ContentStore, NoStore, ReleaseStore};

/// Registry acquisition using [wasm-pkg-client].
///
/// Fetches exact `namespace:name@version` references only, verifying every
/// result against the registry's content digest. The configuration bounds
/// where a package is fetched from: one it routes — by its
/// `package_registry_overrides` entry, its namespace's
/// `namespace_registries` entry, or the `default_registry` — is fetched from
/// that registry alone, and a load naming another is refused; one it routes
/// nowhere is fetched from the registry the load names, and refused when it
/// names none. The attached store is a byte cache and offline fallback —
/// never the authority while the registry is reachable — so a failing store
/// degrades a load, never refuses it.
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

    // The registry `package` is fetched from. The configuration's routing
    // bounds the load: a package it routes is served by that registry alone,
    // and one it routes nowhere by the registry the load names.
    fn registry(
        &self, package: &PackageRef, endpoint: Option<&str>,
    ) -> Result<Registry, AcquireError> {
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
                "no registry routes `{package}`: the load names none, and the deployment's \
                 `registries` routes neither the `{}` namespace nor a default",
                package.namespace()
            ))),
        }
    }

    // A client fetching through the configuration as the deployment declares
    // it, mapping and metadata intact; a package it routes nowhere is routed
    // to `registry` — the one the load named — for this fetch alone. Loads
    // are rare, so a fresh client per fetch beats caching machinery.
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

    /// Resolve and fetch `package`, serving verified bytes from the store
    /// when possible.
    async fn fetch(&self, package: &str, endpoint: Option<&str>) -> Result<Vec<u8>, AcquireError> {
        let (package_ref, version) =
            parse_package(package).map_err(|error| AcquireError::Refused(format!("{error:#}")))?;
        let registry = self.registry(&package_ref, endpoint)?;
        let client = self.client(&package_ref, &registry);
        let registry = registry.to_string();
        let release =
            self.resolve_release(&client, &registry, package, &package_ref, &version).await?;
        let expected: Digest = release.content_digest.to_string().parse().map_err(|error| {
            AcquireError::Refused(format!(
                "the registry digest for `{package}` is unsupported: {error}"
            ))
        })?;

        if let Some(bytes) = self.stored(package, expected).await {
            return Ok(bytes);
        }

        let content = client
            .stream_content(&package_ref, &release)
            .await
            .map_err(|error| AcquireError::Unavailable(format!("fetching `{package}`: {error}")))?;
        let bytes = collect(content)
            .await
            .map_err(|error| AcquireError::Unavailable(format!("reading `{package}`: {error}")))?;

        let resolved = Digest::of(&bytes);
        if resolved != expected {
            // The registry misdelivered; a retry may serve honest bytes.
            return Err(AcquireError::Unavailable(format!(
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
    ) -> Result<Release, AcquireError> {
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
                    .map_err(|error| AcquireError::Unavailable(format!("{error:#}")))?;
                let Some(digest) = stored else {
                    return Err(AcquireError::Unavailable(format!(
                        "resolving `{package}`: {error}"
                    )));
                };
                tracing::warn!(
                    package,
                    registry,
                    error = format!("{error:#}"),
                    "registry unreachable; falling back to the stored release record"
                );
                let content_digest = digest.parse().map_err(|error| {
                    AcquireError::Unavailable(format!(
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
            Err(error) => Err(AcquireError::Refused(format!("resolving `{package}`: {error}"))),
        }
    }
}

impl<S: ContentStore + ReleaseStore> RegistrySource for RegistryClient<S> {
    fn acquire<'a>(
        &'a self, package: &'a str, endpoint: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        self.fetch(package, endpoint).boxed()
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
        let error = client.registry(&package("acme:tool"), None).expect_err("refused");
        assert!(
            matches!(error, AcquireError::Refused(detail) if detail.contains("`acme` namespace"))
        );
    }

    // The configuration bounds the load: an unrouted package takes the
    // registry the load names, a routed one is served by its route alone.
    #[test]
    fn endpoint_precedence() {
        let unrouted = RegistryClient::default();
        let named = unrouted.registry(&package("acme:tool"), Some("ghcr.io")).expect("named");
        assert_eq!(named.to_string(), "ghcr.io");
        let error = unrouted.registry(&package("acme:tool"), Some("not a registry")).expect_err("");
        assert!(
            matches!(error, AcquireError::Refused(detail) if detail.contains("not a valid name"))
        );

        let routed = RegistryClient::from_toml("[namespace_registries]\nacme = \"acme.test\"\n")
            .expect("a routed configuration parses");
        let same = routed.registry(&package("acme:tool"), Some("acme.test")).expect("same route");
        assert_eq!(same.to_string(), "acme.test");
        let error = routed.registry(&package("acme:tool"), Some("ghcr.io")).expect_err("refused");
        assert!(
            matches!(&error, AcquireError::Refused(detail) if detail.contains("routed to `acme.test`")),
            "{error}"
        );
    }

    // The client fetches through the configuration as the deployment
    // declares it — a routed package keeps its mapping, custom metadata and
    // all — and only a package routed nowhere is routed, to the registry the
    // load named, in the client's configuration alone.
    #[test]
    fn client_keeps_routing() {
        let client = RegistryClient::from_toml(
            "[package_registry_overrides]\n\"acme:tool\" = { registry = \"acme.test\", metadata = \
             { preferredProtocol = \"oci\" } }\n",
        )
        .expect("a custom mapping parses");

        let tool = package("acme:tool");
        let routed = client.registry(&tool, None).expect("routed");
        let mapping =
            client.client(&tool, &routed).config().package_registry_override(&tool).cloned();
        assert!(matches!(mapping, Some(RegistryMapping::Custom(_))), "{mapping:?}");

        let other = package("acme:other");
        let named = client.registry(&other, Some("ghcr.io")).expect("named");
        let mapping =
            client.client(&other, &named).config().package_registry_override(&other).cloned();
        assert!(
            matches!(&mapping, Some(RegistryMapping::Registry(registry)) if *registry == named),
            "{mapping:?}"
        );
        assert!(client.config.package_registry_override(&other).is_none());
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
