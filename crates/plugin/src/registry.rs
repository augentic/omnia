//! Registry acquisition, store first, over [wasm-pkg-client] or a fetch held
//! to the loading guest's grants.
//!
//! [wasm-pkg-client]: https://github.com/bytecodealliance/wasm-pkg-tools

mod named;

use std::io;

use anyhow::{Context as _, Result};
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use omnia_core::{AcquireError, Digest, Grants};
use wasm_pkg_client::{Client, Config, ContentStream, PackageRef, Registry, RegistryMapping};
use wasmtime::Engine;

use self::named::Named;
use crate::source::RegistrySource;
use crate::store::{NoStore, PackageStore, Reference};

/// Registry acquisition using [wasm-pkg-client], through a [`PackageStore`].
///
/// Fetches exact `namespace:name@version` references only. The store answers
/// first: a release it holds is served with no network, whoever wrote it. One
/// it lacks is fetched and verified against the registry's content digest.
/// The configuration bounds where a package is fetched from: one it routes —
/// by its `package_registry_overrides` entry, its namespace's
/// `namespace_registries` entry, or the `default_registry` — is fetched from
/// that registry alone, and a load naming another is refused; one it routes
/// nowhere is fetched from the registry the load names, and refused, naming
/// the namespace and the store, when it names none.
///
/// A fetch from the registry a load names is held to the [`Grants`]
/// [`acquire`](RegistrySource::acquire) is passed. Each request it would
/// make fails with [`AcquireError::Denied`], before anything connects, when
/// the grants do not reach its host:
///
/// - the registry's `/.well-known/wasm-pkg/registry.json` metadata;
/// - the manifest and layer, at the OCI host that metadata names;
/// - the token realm a `401` names;
/// - every redirect any of them answers with.
///
/// Such a fetch speaks OCI alone. It authenticates with the deployment's
/// `[registry."<name>".oci]` settings alone, never the process environment
/// or a Docker credential store. A registry the deployment serves with the
/// `local` backend is read from disk and makes no request.
///
/// The store keeps the deployment's word alone. A release the configuration
/// routed is written to the store once — final until removed — while one
/// fetched from the registry a load named is served to that load and kept
/// nowhere, so nothing a caller names becomes what a later acquisition of
/// the reference is served.
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
    ) -> Result<Origin, AcquireError> {
        let package = reference.package();
        let named = endpoint
            .map(|endpoint| {
                endpoint.parse::<Registry>().map_err(|error| {
                    AcquireError::Invalid(format!(
                        "registry `{endpoint}` is not a valid name: {error}"
                    ))
                })
            })
            .transpose()?;
        match (self.config.resolve_registry(package), named) {
            (Some(routed), Some(named)) if *routed != named => Err(AcquireError::Denied(format!(
                "`{package}` is routed to `{routed}` by the deployment's `registries`; it cannot \
                 be fetched from `{named}`"
            ))),
            (Some(routed), _) => Ok(Origin::Routed(routed.clone())),
            (None, Some(named)) => Ok(Origin::Named(named)),
            (None, None) => Err(AcquireError::NotFound(format!(
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
        &self, package: &str, endpoint: Option<&str>, grants: &Grants,
    ) -> Result<Vec<u8>, AcquireError> {
        let reference: Reference = package
            .parse()
            .map_err(|error: anyhow::Error| AcquireError::Invalid(format!("{error:#}")))?;

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

        // then the registry the deployment routes it to, or the load names
        let origin = self.registry(&reference, endpoint)?;
        let bytes = self.fetch(&reference, &origin, grants).await?;

        // SECURITY: only the deployment's routing writes the store; a release
        // a load fetched from a registry it named would otherwise be served to
        // every later acquisition of the reference, a declared guest's included
        let stored = match &origin {
            Origin::Routed(_) => match self.store.put(&reference, &bytes).await {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(
                        package,
                        error = format!("{error:#}"),
                        "failed to store the package"
                    );
                    false
                }
            },
            Origin::Named(_) => false,
        };
        tracing::info!(
            package,
            registry = %origin.registry(),
            digest = %Digest::of(&bytes),
            stored,
            "package fetched from the registry"
        );
        Ok(bytes)
    }

    // Fetch the release and hold its bytes to the registry's digest; raw wasm
    // alone leaves here, so nothing pre-compiled is ever stored.
    async fn fetch(
        &self, reference: &Reference, origin: &Origin, grants: &Grants,
    ) -> Result<Vec<u8>, AcquireError> {
        let (bytes, expected) = match origin {
            Origin::Named(registry) if !self.serves_locally(registry) => {
                Named::new(&self.config, registry, grants)?.fetch(reference).await?
            }
            Origin::Routed(registry) | Origin::Named(registry) => {
                self.pull(reference, registry).await?
            }
        };

        let resolved = Digest::of(&bytes);
        if resolved != expected {
            // the registry misdelivered; a retry may serve honest bytes
            return Err(AcquireError::Unavailable(format!(
                "package `{reference}` content hashes to {resolved}, not the registry digest \
                 {expected}"
            )));
        }
        if Engine::detect_precompiled(&bytes).is_some() {
            return Err(AcquireError::Invalid(format!(
                "package `{reference}` is pre-compiled, but a package admits raw wasm alone"
            )));
        }
        Ok(bytes)
    }

    // Resolve the release and stream its content over wasm-pkg-client.
    async fn pull(
        &self, reference: &Reference, registry: &Registry,
    ) -> Result<(Vec<u8>, Digest), AcquireError> {
        let package = reference.package();
        let client = self.client(package, registry);
        let release = client
            .get_release(package, reference.version())
            .await
            .map_err(|error| release_error(&error, format!("resolving `{reference}`: {error}")))?;
        let expected: Digest = release.content_digest.to_string().parse().map_err(|error| {
            AcquireError::Unsupported(format!(
                "the registry digest for `{reference}` is unsupported: {error}"
            ))
        })?;

        let content = client.stream_content(package, &release).await.map_err(|error| {
            AcquireError::Unavailable(format!("fetching `{reference}`: {error}"))
        })?;
        let bytes = collect(content).await.map_err(|error| {
            AcquireError::Unavailable(format!("reading `{reference}`: {error}"))
        })?;
        Ok((bytes, expected))
    }

    // A registry the deployment serves from disk makes no request the grants
    // could hold.
    fn serves_locally(&self, registry: &Registry) -> bool {
        self.config.registry_config(registry).and_then(|settings| settings.default_backend())
            == Some("local")
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
        &'a self, package: &'a str, endpoint: Option<&'a str>, grants: &'a Grants,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        self.acquire(package, endpoint, grants).boxed()
    }
}

// Whose word a package's registry is: the deployment's, by its `registries`
// routing, or the load's, for a package the deployment routes nowhere.
enum Origin {
    Routed(Registry),
    Named(Registry),
}

impl Origin {
    const fn registry(&self) -> &Registry {
        match self {
            Self::Routed(registry) | Self::Named(registry) => registry,
        }
    }
}

// What a registry's answer to a release lookup says: the release is absent,
// the request is malformed, or the registry failed and a retry may succeed.
fn release_error(error: &wasm_pkg_client::Error, detail: String) -> AcquireError {
    use wasm_pkg_client::Error;

    match error {
        Error::PackageNotFound | Error::VersionNotFound(_) | Error::NoRegistryForNamespace(_) => {
            AcquireError::NotFound(detail)
        }
        Error::RegistryError(source) | Error::RegistryMetadataError(source)
            if is_not_found(source) =>
        {
            AcquireError::NotFound(detail)
        }
        Error::IoError(source) if source.kind() == io::ErrorKind::NotFound => {
            AcquireError::NotFound(detail)
        }
        Error::InvalidPackageRef(_)
        | Error::InvalidVersion(_)
        | Error::InvalidLabel(_)
        | Error::InvalidRegistry(_) => AcquireError::Invalid(detail),
        _ => AcquireError::Unavailable(detail),
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
