//! Registry acquisition over [wasm-pkg-client].
//!
//! [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools

use std::collections::HashMap;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use tokio::sync::Mutex;
use wasm_pkg_client::{Client, Config, ContentStream, PackageRef, Registry, Release, Version};

use crate::store::{
    ContentStore, NoStore, PluginStore, ReleaseRecord, ReleaseStore, sha256_digest,
};

/// Registry acquisition policy — the
/// [`Acquirer::registry`](crate::Acquirer::registry) slot.
pub trait RegistrySource: Send + Sync + 'static {
    /// Produce the raw component bytes for `package` from `registry`
    /// (`None` selects the acquirer's default endpoint).
    fn acquire<'a>(
        &'a self, package: &'a str, registry: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>>>;
}

/// Registry acquisition using [wasm-pkg-client].
///
/// Fetches exact `namespace:name@version` references only, verifying every
/// result against the registry's content digest. The attached [`PluginStore`]
/// is a byte cache and offline fallback — never the authority while the
/// registry is reachable.
///
/// [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools
pub struct RegistryClient<S = NoStore> {
    default_registry: String,
    config: Config,
    store: S,
    clients: Mutex<HashMap<Registry, Client>>,
}

impl RegistryClient<NoStore> {
    /// Cacheless acquirer whose default endpoint is `default_registry`.
    ///
    /// Starts from an empty client configuration — no user-global wasm-pkg
    /// config file and no hard-coded fallback registries — so the compiled
    /// binary alone attests which endpoints the deployment may reach.
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

impl<S: PluginStore> RegistryClient<S> {
    /// Replaces the client configuration (per-registry backend and
    /// credential settings).
    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Attaches a [`PluginStore`] as byte cache and offline fallback.
    #[must_use]
    pub fn cached<S2: PluginStore>(self, store: S2) -> RegistryClient<S2> {
        RegistryClient {
            default_registry: self.default_registry,
            config: self.config,
            store,
            clients: self.clients,
        }
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
    ) -> Result<Release> {
        let full_name = package_ref.to_string();
        match client.get_release(package_ref, version).await {
            Ok(release) => {
                let record = ReleaseRecord {
                    version: version.to_string(),
                    content_digest: release.content_digest.to_string(),
                };
                ReleaseStore::put(&self.store, registry, &full_name, &record)
                    .await
                    .with_context(|| format!("recording `{package}`"))?;
                Ok(release)
            }
            Err(error) if is_network_failure(&error) => {
                let stored =
                    ReleaseStore::get(&self.store, registry, &full_name, &version.to_string())
                        .await?;
                let Some(record) = stored else {
                    return Err(anyhow::Error::new(error).context(format!("resolving `{package}`")));
                };
                tracing::warn!(
                    package,
                    registry,
                    error = format!("{error:#}"),
                    "registry unreachable; falling back to the stored release record"
                );
                let content_digest = record.content_digest.parse().map_err(|error| {
                    anyhow!(
                        "stored release record for `{package}` carries a malformed digest: {error}"
                    )
                })?;
                Ok(Release {
                    version: version.clone(),
                    content_digest,
                })
            }
            Err(error) => Err(anyhow::Error::new(error).context(format!("resolving `{package}`"))),
        }
    }
}

impl<S: PluginStore> RegistrySource for RegistryClient<S> {
    fn acquire<'a>(
        &'a self, package: &'a str, registry: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>>> {
        async move {
            let (package_ref, version) = parse_package(package)?;
            let registry = registry.unwrap_or(&self.default_registry);
            let parsed: Registry = registry
                .parse()
                .map_err(|error| anyhow!("registry `{registry}` is not a valid name: {error}"))?;

            let client = self.client(parsed).await;
            let release =
                self.resolve_release(&client, registry, package, &package_ref, &version).await?;
            let digest = release.content_digest.to_string();

            let stored = ContentStore::get(&self.store, &digest).await?;
            if let Some(bytes) = stored {
                // A poisoned entry must never become code; discard and refetch.
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

            let content = client
                .stream_content(&package_ref, &release)
                .await
                .with_context(|| format!("fetching `{package}`"))?;
            let bytes = collect(content).await.with_context(|| format!("reading `{package}`"))?;

            let resolved = sha256_digest(&bytes);
            ensure!(
                resolved == digest,
                "package `{package}` content hashes to {resolved}, not the registry \
                 digest {digest}"
            );
            ContentStore::put(&self.store, &digest, &bytes)
                .await
                .with_context(|| format!("storing `{package}`"))?;
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
