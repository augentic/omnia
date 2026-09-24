//! Registry acquisition over [wasm-pkg-client].
//!
//! [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools

use anyhow::{Context as _, Result, bail};
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use omnia_core::sha256_digest;
use wasm_pkg_client::{
    Client, Config, ContentStream, PackageRef, Registry, RegistryMapping, Release, Version,
};

use crate::error::LoadError;
use crate::source::RegistrySource;
use crate::store::{ContentStore, NoStore, ReleaseStore};

/// Registry acquisition using [wasm-pkg-client].
///
/// Fetches exact `namespace:name@version` references only, verifying every
/// result against the registry's content digest. A load that names its
/// registry fetches from it; one that names none takes the client
/// configuration's routing — its `package_registry_overrides` entry, its
/// namespace's `namespace_registries` entry, or the `default_registry` — and
/// a package the configuration routes nowhere is refused. The attached store
/// is a byte cache and offline fallback — never the authority while the
/// registry is reachable — so a failing store degrades a load, never refuses
/// it.
///
/// [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools
pub struct RegistryClient<S = NoStore> {
    config: Config,
    store: S,
}

impl RegistryClient<NoStore> {
    /// Cacheless acquirer routing every unaddressed package through `config`.
    ///
    /// The configuration is exactly what the deployment declares — no
    /// user-global wasm-pkg config file and no hard-coded fallback registries
    /// are consulted. It routes the loads that name no registry; a load that
    /// names one fetches from it whatever the configuration says.
    #[must_use]
    pub const fn new(config: Config) -> Self {
        Self {
            config,
            store: NoStore,
        }
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

    /// The registry `package` resolves against: `endpoint` when the load
    /// names one, else the configuration's routing.
    fn registry(
        &self, package: &PackageRef, endpoint: Option<&str>,
    ) -> Result<Registry, LoadError> {
        if let Some(endpoint) = endpoint {
            return endpoint.parse().map_err(|error| {
                LoadError::Refused(format!("registry `{endpoint}` is not a valid name: {error}"))
            });
        }
        self.config.resolve_registry(package).cloned().ok_or_else(|| {
            LoadError::Refused(format!(
                "no registry routes `{package}`: the load names none, and the deployment's \
                 `registries` routes neither the `{}` namespace nor a default",
                package.namespace()
            ))
        })
    }

    /// A client that fetches `package` from `registry`, whatever the
    /// configuration routes the package or its namespace to; loads are rare,
    /// so a fresh client per fetch beats caching machinery.
    fn client(&self, package: &PackageRef, registry: &Registry) -> Client {
        let mut config = self.config.clone();
        config.set_package_registry_override(
            package.clone(),
            RegistryMapping::Registry(registry.clone()),
        );
        Client::new(config)
    }

    /// Resolve and fetch `package`, serving verified bytes from the store
    /// when possible.
    async fn fetch(&self, package: &str, endpoint: Option<&str>) -> Result<Vec<u8>, LoadError> {
        let (package_ref, version) =
            parse_package(package).map_err(|error| LoadError::Refused(format!("{error:#}")))?;
        let registry = self.registry(&package_ref, endpoint)?;
        let client = self.client(&package_ref, &registry);
        let registry = registry.to_string();
        let release =
            self.resolve_release(&client, &registry, package, &package_ref, &version).await?;
        let digest = release.content_digest.to_string();

        if let Some(bytes) = self.stored(package, &digest).await {
            return Ok(bytes);
        }

        let content = client
            .stream_content(&package_ref, &release)
            .await
            .map_err(|error| LoadError::Unavailable(format!("fetching `{package}`: {error}")))?;
        let bytes = collect(content)
            .await
            .map_err(|error| LoadError::Unavailable(format!("reading `{package}`: {error}")))?;

        let resolved = sha256_digest(&bytes);
        if resolved != digest {
            // The registry misdelivered; a retry may serve honest bytes.
            return Err(LoadError::Unavailable(format!(
                "package `{package}` content hashes to {resolved}, not the registry \
                 digest {digest}"
            )));
        }
        if let Err(error) = self.store.put_content(&digest, &bytes).await {
            tracing::warn!(
                package,
                digest,
                error = format!("{error:#}"),
                "failed to store the package content"
            );
        }
        tracing::debug!(package, digest = %resolved, "package acquired");
        Ok(bytes)
    }

    /// The store's verified bytes for `digest`; `None` on a miss, a failed
    /// verification, or an unreadable store — the cache never refuses a load.
    async fn stored(&self, package: &str, digest: &str) -> Option<Vec<u8>> {
        match self.store.content(digest).await {
            Ok(Some(bytes)) => {
                // A poisoned entry must never become code; discard and refetch.
                if sha256_digest(&bytes) == digest {
                    tracing::debug!(package, digest, "package served from the store");
                    Some(bytes)
                } else {
                    tracing::warn!(
                        package,
                        digest,
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
                    digest,
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
    fn acquire<'a>(
        &'a self, package: &'a str, registry: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LoadError>> {
        self.fetch(package, registry).boxed()
    }
}

/// Whether a resolution error is a transport failure — endpoint unreachable,
/// registry misbehaving — rather than an authoritative registry answer
/// (not found, yanked, malformed input), which must never be papered over
/// by a stored record.
const fn is_network_failure(error: &wasm_pkg_client::Error) -> bool {
    matches!(
        error,
        wasm_pkg_client::Error::RegistryError(_)
            | wasm_pkg_client::Error::RegistryMetadataError(_)
            | wasm_pkg_client::Error::IoError(_)
    )
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
