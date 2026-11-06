//! # Deployment manifest (`omnia.toml`)
//!
//! Registry population, routing, and transport are *deployment* decisions,
//! not build-time ones. A manifest may be loaded from a startup file or
//! assembled programmatically before the registry is built.
//!
//! The manifest is parsed **generically** — Omnia sees opaque [`GuestId`]s,
//! never `source:`/`target:`/`mcp`. Consumers write the concrete file; the
//! runtime core stays domain-agnostic. What crosses between guests is not
//! declared here at all: each component says what it imports and exports,
//! and the runtime links every interface outside its own namespaces.
//!
//! The `[[guest]]` population is the allow-list of everything that may run:
//! each entry is a file, embedded-bytes, or package source, named by its
//! `name` or, absent one, by its file's stem or its package reference
//! without the version. Bytes the process already holds load at boot; a
//! path or package loads at its first use — the first time a route, the
//! command drive, a link call, a host dispatch, or the guest loader names
//! it. An entry may pin the `digest` its bytes must hash to; the format it
//! admits follows from the source (see [`SourceSpec`]). Each guest's
//! `routes` tables and the `[plugins]` table — the package store and the
//! `registries` configuration that routes package sources — are consumed
//! too. Distributed `[transport]` is not yet implemented: only the
//! in-process default is accepted.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use omnia_core::{
    CliRoutes, Digest, GuestId, HttpRoutes, PatternRoutes, ResolvedPreopen, Routes, Source,
    SourceSpec,
};
use serde::Deserialize;

/// The optional capabilities this build compiled in; a manifest may only
/// declare what the build serves.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    /// The `loader` feature (the `[plugins]` table and package sources).
    pub loader: bool,
}

impl Features {
    /// The features this build was compiled with.
    pub const COMPILED: Self = Self {
        loader: cfg!(feature = "loader"),
    };
}

/// The deployment manifest: every guest that may run, what they mount, and
/// where packages are kept and fetched from.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Manifest {
    /// Registry population: each entry maps an identity to a source.
    #[serde(rename = "guest")]
    pub guests: Vec<GuestEntry>,
    /// Working-tree mounts preopened into the guest sandbox.
    #[serde(rename = "mount")]
    pub mounts: Vec<Mount>,
    /// The package store and the registry routing every package — a
    /// declared `source.package` or one a guest loads — is acquired through;
    /// absent, every package is refused.
    pub plugins: Option<PluginsConfig>,
    /// Transport configuration for host-mediated calls.
    pub transport: Transport,
}

impl Manifest {
    /// Start an empty programmatic manifest.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a manifest and resolve its relative paths against its directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the current directory cannot be read, or the file
    /// cannot be read or parsed as TOML.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        let mut manifest: Self = toml::from_str(&text)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        manifest.resolve_names();
        manifest.resolve_paths(base);
        Ok(manifest)
    }

    /// Create a single-guest manifest from a component file, read now and
    /// named by the file's stem, so the guest loads at boot.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    pub fn from_wasm(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes =
            fs::read(path).with_context(|| format!("reading component {}", path.display()))?;
        let name = GuestId::from_path(&path.to_string_lossy());
        Ok(Self::new().guest(GuestEntry::new(name.as_str(), bytes)))
    }

    /// Append a guest.
    #[must_use]
    pub fn guest(mut self, guest: GuestEntry) -> Self {
        self.guests.push(guest);
        self
    }

    /// Append workspace mounts.
    #[must_use]
    pub fn mounts(mut self, mounts: impl IntoIterator<Item = Mount>) -> Self {
        self.mounts.extend(mounts);
        self
    }

    /// Set the `[plugins]` table: the package store and the registry routing.
    #[must_use]
    pub fn plugins(mut self, config: PluginsConfig) -> Self {
        self.plugins = Some(config);
        self
    }

    /// Set where the wasm-pkg client configuration comes from (the manifest's
    /// `[plugins] registries`): the routing of every package.
    #[must_use]
    pub fn registries(mut self, config: impl Into<RegistryConfig>) -> Self {
        self.plugins.get_or_insert_default().registries = Some(config.into());
        self
    }

    /// Set the package store (the manifest's `[plugins] store`): the
    /// directory every acquired package is read from first and written to
    /// once.
    #[must_use]
    pub fn store(mut self, root: impl Into<PathBuf>) -> Self {
        self.plugins.get_or_insert_default().store = Some(root.into());
        self
    }

    /// Validate manifest-level invariants surfaced before the registry is
    /// built. An `allow_empty` (dynamic) deployment may define no `[[guest]]`
    /// entry; `features` is what the manifest may declare.
    ///
    /// # Errors
    ///
    /// Returns an error if a `[[guest]]` entry names no guest or repeats a
    /// name, no entry exists when `allow_empty` is false, more than one entry
    /// is the command guest, the transport is not in-process, or the manifest
    /// declares `[plugins]` or a package source that `features` lacks the
    /// `loader` to serve.
    pub fn validate(&self, allow_empty: bool, features: Features) -> Result<()> {
        let mut names = BTreeSet::new();
        for entry in &self.guests {
            if entry.name.is_empty() {
                bail!(
                    "a [[guest]] entry names no guest ({:?}): set `name`, or give a `source.path` \
                     whose file stem names it, or a `source.package` whose reference names it",
                    entry.source
                );
            }
            if !names.insert(entry.name.as_str()) {
                bail!("duplicate [[guest]] name `{}`: guest names must be unique", entry.name);
            }
        }
        if !allow_empty && self.guests.is_empty() {
            bail!("manifest defines no [[guest]] entry");
        }
        let marked: Vec<&str> =
            self.guests.iter().filter(|e| e.command).map(|e| e.name.as_str()).collect();
        if marked.len() > 1 {
            bail!(
                "multiple [[guest]] entries marked `command = true` ({}): at most one guest may \
                 be the command guest",
                marked.join(", ")
            );
        }
        if self.transport.default != TransportKind::InProcess {
            bail!(
                "transport `{:?}` is not yet implemented; only in-process transport is supported",
                self.transport.default
            );
        }
        // refuse policy the compiled runtime cannot serve, rather than never install it
        if !features.loader {
            if self.plugins.is_some() {
                bail!(
                    "this runtime was built without the `loader` feature; remove `plugins` or \
                     enable the feature on the `omnia` dependency (`features = [\"loader\"]`)"
                );
            }
            if let Some((entry, package)) =
                self.guests.iter().find_map(|entry| match &entry.source {
                    SourceSpec::Package(package) => Some((entry, package)),
                    SourceSpec::Path(_) | SourceSpec::Bytes(_) => None,
                })
            {
                bail!(
                    "guest `{}` is the package `{package}`, but this runtime was built without \
                     the `loader` feature; give it a `source.path` or enable the feature on the \
                     `omnia` dependency (`features = [\"loader\"]`)",
                    entry.name
                );
            }
        }
        Ok(())
    }

    // A `[[guest]]` that names no guest is named by its file's stem or its
    // package reference without the version.
    fn resolve_names(&mut self) {
        for guest in &mut self.guests {
            if !guest.name.is_empty() {
                continue;
            }
            match &guest.source {
                SourceSpec::Path(path) => {
                    guest.name = GuestId::from_path(&path.to_string_lossy()).as_str().into();
                }
                SourceSpec::Package(reference) => {
                    guest.name = GuestId::from_package(reference).as_str().into();
                }
                SourceSpec::Bytes(_) => {}
            }
        }
    }

    fn resolve_paths(&mut self, base: &Path) {
        for guest in &mut self.guests {
            if let SourceSpec::Path(path) = &mut guest.source
                && path.is_relative()
            {
                *path = base.join(&*path);
            }
        }
        for mount in &mut self.mounts {
            if mount.path.is_relative() && !is_home_relative(&mount.path) {
                mount.path = base.join(&mount.path);
            }
        }
        if let Some(plugins) = &mut self.plugins {
            if let Some(RegistryConfig::Path(path)) = &mut plugins.registries
                && path.is_relative()
            {
                *path = base.join(&*path);
            }
            if let Some(store) = &mut plugins.store
                && store.is_relative()
                && !is_home_relative(store)
            {
                *store = base.join(&*store);
            }
        }
    }

    /// Expand a leading `~/` in every mount path and the package store
    /// against `$HOME` — `~/` alone, never `~user/` — so a deployment names
    /// the operator's home without knowing it.
    ///
    /// # Errors
    ///
    /// Returns an error if a path starts with `~/` and `$HOME` is unset.
    pub fn expand_home(&mut self) -> Result<()> {
        let mut home = None;
        let mut expand = |path: &mut PathBuf| -> Result<()> {
            let Ok(rest) = path.strip_prefix("~") else {
                return Ok(());
            };
            let home =
                match &home {
                    Some(home) => home,
                    None => home.insert(std::env::var_os("HOME").map(PathBuf::from).with_context(
                        || format!("expanding `{}`: HOME is unset", path.display()),
                    )?),
                };
            *path = home.join(rest);
            Ok(())
        };
        for mount in &mut self.mounts {
            expand(&mut mount.path)?;
        }
        if let Some(store) = self.plugins.as_mut().and_then(|plugins| plugins.store.as_mut()) {
            expand(store)?;
        }
        Ok(())
    }

    /// The wasm-pkg client configuration as TOML text: a `Path` read now, or
    /// the `Contents` as carried; `None` when the manifest declares none.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is a `Path` that cannot be read.
    pub fn registry_config(&self) -> Result<Option<String>> {
        self.plugins
            .as_ref()
            .and_then(|plugins| plugins.registries.as_ref())
            .map(|config| match config {
                RegistryConfig::Path(path) => fs::read_to_string(path).with_context(|| {
                    format!("reading the `registries` configuration {}", path.display())
                }),
                RegistryConfig::Contents(contents) => Ok(contents.to_string()),
            })
            .transpose()
    }

    /// The package store's directory, as the manifest names it; `None` when
    /// it names none, where a fetched package is kept nowhere.
    #[must_use]
    pub fn store_root(&self) -> Option<&Path> {
        self.plugins.as_ref().and_then(|plugins| plugins.store.as_deref())
    }

    /// Resolve every `[[guest]]` into its loadable source, in declaration
    /// order; the deployment loads the embedded ones at boot and the rest at
    /// first use.
    #[must_use]
    pub fn sources(&self) -> Vec<Source> {
        self.guests.iter().map(source).collect()
    }

    /// Per-trigger route tables aggregated from each guest's `routes` lists,
    /// in guest declaration order.
    #[must_use]
    pub fn routes(&self) -> Routes {
        let pairs = |select: fn(&GuestRoutes) -> &Vec<String>| {
            self.guests.iter().flat_map(move |guest| {
                select(&guest.routes)
                    .iter()
                    .map(|pattern| (pattern.clone(), GuestId::from(guest.name.as_str())))
            })
        };
        let http = HttpRoutes::new(pairs(|routes| &routes.http));
        let messaging = PatternRoutes::new(pairs(|routes| &routes.messaging));
        let websocket = PatternRoutes::new(pairs(|routes| &routes.websocket));
        // cli routes are not yet parsed; empty makes a sole `wasi:cli/run` exporter the catch-all
        Routes::new(http, messaging, websocket, CliRoutes::default())
    }

    /// The identity of the guest marked `command = true`, if any.
    #[must_use]
    pub fn command_guest(&self) -> Option<GuestId> {
        self.guests.iter().find(|e| e.command).map(|e| GuestId::from(e.name.as_str()))
    }

    /// Resolve every `[[mount]]` into a [`ResolvedPreopen`].
    #[must_use]
    pub fn preopens(&self) -> Vec<ResolvedPreopen> {
        self.mounts.iter().map(|entry| entry.resolve(Path::new("."))).collect()
    }
}

/// The `[plugins]` table: where acquired packages are kept and how they are
/// routed to a registry.
///
/// Every package — a declared `source.package` at its first use, a package a
/// guest loads — is read from the `store` first and, when it is not there,
/// fetched from the registry `registries` routes it to and written to the
/// store once. A deployment naming neither refuses every package.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginsConfig {
    /// The package store: one directory, each release filed as
    /// `namespace_name@version.wasm`. A leading `~/` is the operator's home;
    /// [`Manifest::load`] resolves another relative path against the
    /// manifest's directory. It need not exist until a package is written,
    /// and may not lie beneath a writable mount. Absent, a fetched package is
    /// kept nowhere.
    pub store: Option<PathBuf>,
    /// Where the wasm-pkg client configuration that routes packages comes
    /// from; absent, a package the store does not hold is refused.
    pub registries: Option<RegistryConfig>,
}

/// Where the wasm-pkg client configuration comes from: the `[plugins]`
/// table of a manifest names a `registries.path`; the `runtime!` macro and
/// the programmatic API carry the `contents`.
///
/// The configuration is the schema of `wkg`'s own `config.toml`: a
/// `default_registry`, `namespace_registries` and `package_registry_overrides`
/// routing past it, and per-registry backend settings. It alone routes a
/// package to a registry; one it routes nowhere, and the store lacks, is
/// refused.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RegistryConfig {
    /// A TOML file. [`Manifest::load`] resolves a relative path against the
    /// manifest's directory; a relative path set programmatically resolves
    /// against the process working directory.
    Path(PathBuf),
    /// The TOML text itself (typically an `include_str!`). TOML cannot
    /// express this variant; it is set through the `runtime!` macro or the
    /// programmatic API.
    #[serde(skip)]
    Contents(Cow<'static, str>),
}

impl RegistryConfig {
    /// The configuration as TOML text.
    pub fn contents(value: impl Into<Cow<'static, str>>) -> Self {
        Self::Contents(value.into())
    }
}

impl From<&'static str> for RegistryConfig {
    fn from(contents: &'static str) -> Self {
        Self::Contents(Cow::Borrowed(contents))
    }
}

impl From<String> for RegistryConfig {
    fn from(contents: String) -> Self {
        Self::Contents(Cow::Owned(contents))
    }
}

impl From<PathBuf> for RegistryConfig {
    fn from(path: PathBuf) -> Self {
        Self::Path(path)
    }
}

impl From<&Path> for RegistryConfig {
    fn from(path: &Path) -> Self {
        Self::Path(path.to_path_buf())
    }
}

/// A single workspace mount: a host directory preopened into the guest
/// sandbox under a guest-visible name.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Mount {
    /// Guest-visible name `preopens.get-directories()` returns (e.g. `.`).
    pub name: String,
    /// Host path. [`Manifest::load`] resolves relative paths against the
    /// manifest's directory; a relative path set programmatically resolves
    /// against the process working directory.
    pub path: PathBuf,
    /// Read+write when `true`; read-only (the review-flow default) otherwise.
    #[serde(default)]
    pub writable: bool,
}

impl Mount {
    /// Resolve this mount into a [`ResolvedPreopen`], joining a relative host
    /// path against `base` (an absolute path passes through unchanged).
    #[must_use]
    pub fn resolve(&self, base: &Path) -> ResolvedPreopen {
        let host_path =
            if self.path.is_absolute() { self.path.clone() } else { base.join(&self.path) };
        ResolvedPreopen::new(self.name.clone(), host_path, self.writable)
    }
}

/// A single registry population entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestEntry {
    /// The guest's name: the [`GuestId`] it registers under, dispatches by,
    /// and is loaded as. Absent in a manifest file, the source path's file
    /// stem (`./guests/echo.wasm` is `echo`) or the package reference without
    /// its version (`acme:tool@1.2.3` is `acme:tool`) names the guest.
    #[serde(default)]
    pub name: String,
    /// Where the guest's component bytes come from, which decides when it
    /// loads: embedded bytes at boot, a path or package at first use.
    pub source: SourceSpec,
    /// Inbound routes targeting this guest, one list per trigger.
    #[serde(default)]
    pub routes: GuestRoutes,
    /// Marks this guest as the command-mode `wasi:cli/run` target; without a
    /// marked guest the sole exporter among the guests loaded at boot is the
    /// catch-all.
    #[serde(default)]
    pub command: bool,
    /// The `sha256:<hex>` the source's bytes must hash to, checked when they
    /// become a guest. A path read at first use admits `omnia compile`
    /// output only when pinned.
    #[serde(default)]
    pub digest: Option<Digest>,
}

impl GuestEntry {
    /// Create a named guest from a local component path, embedded component
    /// bytes, or a package reference.
    #[must_use]
    pub fn new(name: impl Into<String>, source: impl Into<SourceSpec>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            routes: GuestRoutes::default(),
            command: false,
            digest: None,
        }
    }

    /// Create a guest from component bytes embedded in the host binary, named
    /// by the stem of the file they were read from.
    ///
    /// The `runtime!` macro's `guests: [{ path: .. }]` lowers to this call,
    /// `include_bytes!` supplying the bytes.
    #[must_use]
    pub fn embedded(path: &str, bytes: &'static [u8]) -> Self {
        Self::new(GuestId::from_path(path).as_str(), bytes)
    }

    /// Create a guest from a package reference, named by the reference
    /// without its version (`acme:tool@1.2.3` is `acme:tool`).
    ///
    /// The `runtime!` macro's `guests: [{ package: .. }]` lowers to this call
    /// when the entry sets no `name`.
    #[must_use]
    pub fn package(reference: &str) -> Self {
        Self::new(GuestId::from_package(reference).as_str(), SourceSpec::package(reference))
    }

    /// Append an HTTP prefix route targeting this guest.
    #[must_use]
    pub fn route_http(mut self, prefix: impl Into<String>) -> Self {
        self.routes.http.push(prefix.into());
        self
    }

    /// Append a messaging topic route targeting this guest.
    #[must_use]
    pub fn route_messaging(mut self, topic: impl Into<String>) -> Self {
        self.routes.messaging.push(topic.into());
        self
    }

    /// Append a WebSocket route targeting this guest.
    #[must_use]
    pub fn route_websocket(mut self, route: impl Into<String>) -> Self {
        self.routes.websocket.push(route.into());
        self
    }

    /// Mark this guest as the command-mode `wasi:cli/run` target.
    #[must_use]
    pub const fn command(mut self) -> Self {
        self.command = true;
        self
    }

    /// Require the source's bytes to hash to `digest`.
    #[must_use]
    pub const fn digest(mut self, digest: Digest) -> Self {
        self.digest = Some(digest);
        self
    }
}

/// A guest's inbound routes, one pattern list per trigger; the containing
/// guest is the implicit target.
///
/// `deny_unknown_fields` turns a misspelled trigger (`routes.grpc = [...]`)
/// into a loud parse error rather than a silent no-op.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuestRoutes {
    /// HTTP path prefixes, matched by longest prefix.
    pub http: Vec<String>,
    /// Messaging topic patterns (`.`-tokenised, `*` one token, `>` trailing
    /// tokens).
    pub messaging: Vec<String>,
    /// WebSocket route patterns (same syntax as messaging).
    pub websocket: Vec<String>,
}

impl GuestRoutes {
    /// Whether no trigger routes to the guest.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.http.is_empty() && self.messaging.is_empty() && self.websocket.is_empty()
    }
}

/// Transport configuration for host-mediated calls.
///
/// Only the in-process default is implemented; manifest validation rejects any
/// other value.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transport {
    /// The transport used for host-mediated calls.
    pub default: TransportKind,
}

/// A transport mechanism for host-mediated calls.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    /// In-process in-memory routing — the co-located default (the only implemented kind).
    #[default]
    InProcess,
    /// Unix-domain socket (same node, separate processes).
    Unix,
    /// NATS (cross-node).
    Nats,
    /// QUIC (cross-node).
    Quic,
}

// a path the deployment names relative to the operator's home, expanded at
// assembly rather than against the manifest's directory
fn is_home_relative(path: &Path) -> bool {
    path.starts_with("~")
}

// The one resolution from a `[[guest]]` entry to its loadable source.
fn source(entry: &GuestEntry) -> Source {
    let source = Source::new(entry.name.as_str(), entry.source.clone());
    match entry.digest {
        Some(digest) => source.pinned(digest),
        None => source,
    }
}
