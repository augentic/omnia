//! Registry acquisition over [wasm-pkg-client], fresh-release-preferred.
//!
//! [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context as _, anyhow, bail};
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use tokio::sync::Mutex;
use wasm_pkg_client::{Client, Config, ContentStream, PackageRef, Registry, Release, Version};

use crate::store::{DirStore, NoStore, PluginStore, ReleaseRecord, sha256_digest};
use crate::{Acquire, AcquireContext, AcquireError, Location};

/// Registry acquisition for the plugin loader over [wasm-pkg-client].
///
/// Fetches exact package versions (`namespace:name@version` — remote lookup
/// never resolves "latest") and verifies every result against the registry's
/// content digest before returning bytes. Serves [`Location::Registry`] only;
/// compose with a path acquirer through [`AcquireExt::or`](crate::AcquireExt::or)
/// for path locations. The operator's own sha256 pin is verified host-side by
/// the loader, after acquisition.
///
/// Resolution is fresh-release-preferred: the release resolves against the
/// registry on every load, refreshing the [`PluginStore`]'s record, and only
/// a *network* failure with a stored record falls back (logged) to the store;
/// content is served from the store by digest — verified against the fresh
/// digest, with a poisoned entry discarded and refetched — or fetched,
/// verified, and persisted. The [`NoStore`] default does none of that: every
/// load resolves and fetches fresh.
///
/// [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools
pub struct RegistryAcquire<S = NoStore> {
    default_registry: String,
    config: Config,
    store: S,
    // One client per effective registry: `Client` resolves endpoints from
    // its `Config`, not per call, so each endpoint override needs its own.
    clients: Mutex<HashMap<Registry, Client>>,
}

impl RegistryAcquire<NoStore> {
    /// Acquirer whose default endpoint is `default_registry`
    /// (a `Location::Registry(None)` load resolves there).
    ///
    /// Starts from an empty client configuration — no user-global wasm-pkg
    /// config file and no hard-coded fallback registries — so the compiled
    /// binary alone attests which endpoints the deployment may reach.
    /// Cacheless until [`cached`](Self::cached) attaches a store; an invalid
    /// registry name refuses as a typed failure at first use.
    #[must_use]
    pub fn new(default_registry: impl Into<String>) -> Self {
        Self {
            default_registry: default_registry.into(),
            config: Config::empty(),
            store: NoStore,
            clients: Mutex::new(HashMap::new()),
        }
    }
}

impl<S: PluginStore> RegistryAcquire<S> {
    /// Replaces the client configuration (per-registry backend and
    /// credential settings). The acquirer's default registry and per-load
    /// overrides still take precedence over the configuration's own default.
    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Attaches a [`PluginStore`]; fetched content persists
    /// verify-before-persist, release records refresh on every reachable
    /// resolution, and a network failure falls back to the stored record.
    #[must_use]
    pub fn cached<S2: PluginStore>(self, store: S2) -> RegistryAcquire<S2> {
        RegistryAcquire {
            default_registry: self.default_registry,
            config: self.config,
            store,
            clients: self.clients,
        }
    }

    /// Attaches a [`DirStore`] rooted at `root` (created lazily).
    #[must_use]
    pub fn cached_at(self, root: impl Into<PathBuf>) -> RegistryAcquire<DirStore> {
        self.cached(DirStore::new(root))
    }

    async fn client(&self, registry: Registry) -> Client {
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(&registry) {
            return client.clone();
        }
        let mut config = self.config.clone();
        config.set_default_registry(Some(registry.clone()));
        let client = Client::new(config);
        clients.insert(registry, client.clone());
        client
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
                let record = ReleaseRecord {
                    version: version.to_string(),
                    content_digest: release.content_digest.to_string(),
                };
                self.store.put_release(registry, &full_name, &record).await.map_err(|error| {
                    AcquireError::Failed(error.context(format!("recording `{package}`")))
                })?;
                Ok(release)
            }
            Err(error) if is_network_failure(&error) => {
                let stored = self
                    .store
                    .get_release(registry, &full_name, &version.to_string())
                    .await
                    .map_err(AcquireError::Failed)?;
                let Some(record) = stored else {
                    return Err(AcquireError::Failed(
                        anyhow::Error::new(error).context(format!("resolving `{package}`")),
                    ));
                };
                tracing::warn!(
                    package,
                    registry,
                    error = format!("{error:#}"),
                    "registry unreachable; falling back to the stored release record"
                );
                let content_digest = record.content_digest.parse().map_err(|error| {
                    AcquireError::Failed(anyhow!(
                        "stored release record for `{package}` carries a malformed digest: {error}"
                    ))
                })?;
                Ok(Release {
                    version: version.clone(),
                    content_digest,
                })
            }
            Err(error) => Err(AcquireError::Failed(
                anyhow::Error::new(error).context(format!("resolving `{package}`")),
            )),
        }
    }
}

impl<S: PluginStore> Acquire for RegistryAcquire<S> {
    fn acquire<'a>(
        &'a self, package: &'a str, from: &'a Location, _context: &'a AcquireContext,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        async move {
            let Location::Registry(endpoint) = from else {
                return Err(AcquireError::Unsupported(format!(
                    "RegistryAcquire serves registry locations only; acquiring `{package}` \
                     from {from} requires a path acquirer such as MountAcquire"
                )));
            };
            let (package_ref, version) = parse_package(package).map_err(AcquireError::Failed)?;
            let registry = endpoint.as_deref().unwrap_or(&self.default_registry);
            let parsed: Registry = registry.parse().map_err(|error| {
                AcquireError::Failed(anyhow!("registry `{registry}` is not a valid name: {error}"))
            })?;

            let client = self.client(parsed).await;
            let release =
                self.resolve_release(&client, registry, package, &package_ref, &version).await?;
            let digest = release.content_digest.to_string();

            // The store serves content by digest — verified against the
            // fresh digest, so a poisoned or truncated entry is discarded
            // (overwritten below) instead of becoming code.
            let stored = self.store.get_content(&digest).await.map_err(AcquireError::Failed)?;
            if let Some(bytes) = stored {
                if sha256_digest(&bytes) == digest {
                    tracing::debug!(package, digest, "package served from the store");
                    return Ok(bytes);
                }
                tracing::warn!(
                    package,
                    digest,
                    "stored content failed verification; discarding and refetching"
                );
            }

            let content = client.stream_content(&package_ref, &release).await.map_err(|error| {
                AcquireError::Failed(
                    anyhow::Error::new(error).context(format!("fetching `{package}`")),
                )
            })?;
            let bytes = collect(content).await.map_err(|error| {
                AcquireError::Failed(
                    anyhow::Error::new(error).context(format!("reading `{package}`")),
                )
            })?;

            let resolved = sha256_digest(&bytes);
            if resolved != digest {
                return Err(AcquireError::Failed(anyhow!(
                    "package `{package}` content hashes to {resolved}, not the registry \
                     digest {digest}"
                )));
            }
            self.store.put_content(&digest, &bytes).await.map_err(|error| {
                AcquireError::Failed(error.context(format!("storing `{package}`")))
            })?;
            tracing::debug!(package, digest = %resolved, "package acquired");
            Ok(bytes)
        }
        .boxed()
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
fn parse_package(package: &str) -> anyhow::Result<(PackageRef, Version)> {
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
