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
//! The `[[guest]]` population (file or embedded-bytes sources, each named
//! by its `name` or, absent one, by its file's stem), each guest's `routes`
//! tables, and the `[registries]` configuration the guest loader routes
//! package loads through are all consumed. Distributed `[transport]` is not
//! yet implemented: only the in-process default is accepted.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use omnia_core::{CliRoutes, GuestId, HttpRoutes, PatternRoutes, ResolvedPreopen, Routes};
use serde::Deserialize;

use super::source::Source;

/// The deployment manifest: which guests load, what they mount, and where
/// later guests may come from.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Manifest {
    /// Registry population: each entry maps an identity to a source.
    #[serde(rename = "guest")]
    pub guests: Vec<GuestEntry>,
    /// Working-tree mounts preopened into the guest sandbox — also the roots
    /// the guest loader resolves path loads against.
    #[serde(rename = "mount")]
    pub mounts: Vec<Mount>,
    /// Where the wasm-pkg client configuration the guest loader routes
    /// package loads through comes from — the default routing a load may
    /// name a registry past; absent, a package load names its registry or
    /// refuses.
    pub registries: Option<RegistryConfig>,
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

    /// Create a single-guest manifest from a component path, named by the
    /// file's stem.
    #[must_use]
    pub fn from_wasm(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = GuestId::from_path(&path.to_string_lossy());
        Self::new().guest(GuestEntry::new(name.as_str(), path))
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

    /// Set where the wasm-pkg client configuration comes from (the manifest's
    /// `[registries]` table): the default routing of a package load.
    #[must_use]
    pub fn registries(mut self, config: impl Into<RegistryConfig>) -> Self {
        self.registries = Some(config.into());
        self
    }

    /// Validate manifest-level invariants surfaced before the registry is
    /// built. An `allow_empty` (dynamic) deployment may define no `[[guest]]`
    /// entries.
    pub(super) fn validate(&self, allow_empty: bool) -> Result<()> {
        if self.guests.is_empty() && !allow_empty {
            bail!("manifest defines no [[guest]] entries");
        }
        let mut names = BTreeSet::new();
        for entry in &self.guests {
            if entry.name.is_empty() {
                bail!(
                    "a [[guest]] entry names no guest ({:?}): set `name`, or give a `source.path` \
                     whose file stem names it",
                    entry.source
                );
            }
            if !names.insert(entry.name.as_str()) {
                bail!("duplicate [[guest]] name `{}`: guest names must be unique", entry.name);
            }
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
        // A manifest can declare policy the compiled runtime cannot serve;
        // refuse up front rather than silently never installing it.
        #[cfg(not(feature = "loader"))]
        if self.registries.is_some() {
            bail!(
                "this runtime was built without the `loader` feature; remove `registries` or \
                 enable the feature on the `omnia` dependency (`features = [\"loader\"]`)"
            );
        }
        Ok(())
    }

    // A `[[guest]]` that names no guest is named by its file's stem, the way a
    // path load registers.
    fn resolve_names(&mut self) {
        for guest in &mut self.guests {
            if guest.name.is_empty()
                && let SourceSpec::Path(path) = &guest.source
            {
                guest.name = GuestId::from_path(&path.to_string_lossy()).as_str().into();
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
            if mount.path.is_relative() {
                mount.path = base.join(&mount.path);
            }
        }
        if let Some(RegistryConfig::Path(path)) = &mut self.registries
            && path.is_relative()
        {
            *path = base.join(&*path);
        }
    }

    /// The wasm-pkg client configuration as TOML text: a `Path` read now, or
    /// the `Contents` as carried; `None` when the manifest declares none.
    pub(super) fn registry_config(&self) -> Result<Option<String>> {
        self.registries
            .as_ref()
            .map(|config| match config {
                RegistryConfig::Path(path) => fs::read_to_string(path).with_context(|| {
                    format!("reading the `registries` configuration {}", path.display())
                }),
                RegistryConfig::Contents(contents) => Ok(contents.to_string()),
            })
            .transpose()
    }

    /// Resolve every `[[guest]]` source into a loadable source.
    ///
    /// # Errors
    ///
    /// Returns an error if a guest uses a source kind not yet supported.
    pub fn sources(&self) -> Result<Vec<Source>> {
        let mut sources = Vec::with_capacity(self.guests.len());
        for entry in &self.guests {
            let id = GuestId::from(entry.name.as_str());
            match &entry.source {
                SourceSpec::Path(path) => sources.push(Source::with_id(id, path)),
                SourceSpec::Bytes(bytes) => sources.push(Source::embedded(id, bytes.clone())),
                SourceSpec::Oci(reference) => {
                    bail!("guest `{id}`: OCI source `{reference}` is not yet supported")
                }
            }
        }
        Ok(sources)
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
        // CLI routes are not yet parsed; an empty table makes a sole
        // `wasi:cli/run` exporter the catch-all (multi-command routing is
        // deferred).
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

/// Where the wasm-pkg client configuration comes from: the `[registries]`
/// table of a manifest names a `path`; the `runtime!` macro and the
/// programmatic API carry the `contents`.
///
/// The configuration is the schema of `wkg`'s own `config.toml`: a
/// `default_registry`, `namespace_registries` and `package_registry_overrides`
/// routing past it, and per-registry backend settings. It routes the package
/// loads that name no registry of their own — one it routes nowhere is
/// refused — while a load that names its registry fetches from it.
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
/// sandbox under a guest-visible name, and a root the guest loader resolves
/// path loads against.
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
    /// stem names the guest (`./guests/echo.wasm` is `echo`).
    #[serde(default)]
    pub name: String,
    /// Where the guest's component bytes come from.
    pub source: SourceSpec,
    /// Inbound routes targeting this guest, one list per trigger.
    #[serde(default)]
    pub routes: GuestRoutes,
    /// Marks this guest as the command-mode `wasi:cli/run` target; without a
    /// marked guest the sole static exporter is the catch-all.
    #[serde(default)]
    pub command: bool,
}

impl GuestEntry {
    /// Create a named guest from a local component path or embedded component bytes.
    #[must_use]
    pub fn new(name: impl Into<String>, source: impl Into<SourceSpec>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            routes: GuestRoutes::default(),
            command: false,
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
}

/// Where a guest's component bytes come from.
///
/// Modelled as an externally tagged enum so TOML's `source.path = "..."` and
/// `source.oci = "..."` each select a variant.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceSpec {
    /// A local `.wasm` / pre-compiled `.bin` path. [`Manifest::load`]
    /// resolves relative paths against the manifest's directory; a relative
    /// path set programmatically resolves against the process working directory.
    Path(PathBuf),
    /// Component bytes embedded in the host binary (typically an
    /// `include_bytes!` blob). TOML cannot express this variant; it is set
    /// through the `runtime!` macro or the programmatic [`GuestEntry`] API.
    #[serde(skip)]
    Bytes(Cow<'static, [u8]>),
    /// A digest-pinned OCI reference. Accepted by the parser and surfaced in the
    /// "not yet supported" error; the puller that consumes it lands as a
    /// follow-up.
    Oci(String),
}

// Manual: the derived impl would dump the embedded component bytes.
impl std::fmt::Debug for SourceSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Self::Bytes(bytes) => write!(f, "Bytes({} bytes)", bytes.len()),
            Self::Oci(reference) => f.debug_tuple("Oci").field(reference).finish(),
        }
    }
}

impl From<&str> for SourceSpec {
    fn from(path: &str) -> Self {
        Self::Path(PathBuf::from(path))
    }
}

impl From<String> for SourceSpec {
    fn from(path: String) -> Self {
        Self::Path(PathBuf::from(path))
    }
}

impl From<&Path> for SourceSpec {
    fn from(path: &Path) -> Self {
        Self::Path(path.to_path_buf())
    }
}

impl From<PathBuf> for SourceSpec {
    fn from(path: PathBuf) -> Self {
        Self::Path(path)
    }
}

impl From<&'static [u8]> for SourceSpec {
    fn from(bytes: &'static [u8]) -> Self {
        Self::Bytes(Cow::Borrowed(bytes))
    }
}

// `include_bytes!` yields `&[u8; N]`, so the array form is the one embedders hit.
impl<const N: usize> From<&'static [u8; N]> for SourceSpec {
    fn from(bytes: &'static [u8; N]) -> Self {
        Self::Bytes(Cow::Borrowed(bytes))
    }
}

impl From<Vec<u8>> for SourceSpec {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Bytes(Cow::Owned(bytes))
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

// Unit tests by design: manifest parsing/validation is pure translation.
#[cfg(test)]
mod tests {
    use omnia_core::Resolver as _;

    use super::*;

    #[test]
    fn parse_multi_guest() {
        let toml = r#"
            [[guest]]
            name = "workflow"
            source.path = "./guests/workflow.wasm"

            [[guest]]
            name = "mcp"
            source.path = "./guests/mcp.wasm"

            [transport]
            default = "in-process"
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.guests.len(), 2);
        assert_eq!(manifest.guests[0].name, "workflow");
        assert!(matches!(manifest.guests[1].source, SourceSpec::Path(_)));
        assert_eq!(manifest.transport.default, TransportKind::InProcess);
    }

    // Nothing declares what crosses between guests; a `services` list is an
    // unknown key like any other.
    #[test]
    fn reject_services_key() {
        let toml = "services = [\"omnia:shared/log\"]\n\n\
             [[guest]]\nname = \"a\"\nsource.path = \"./a.wasm\"\n";
        toml::from_str::<Manifest>(toml).unwrap_err();
    }

    #[test]
    fn reject_unknown_keys() {
        let toml = "bogus = 1\n\n[[guest]]\nname = \"a\"\nsource.path = \"./a.wasm\"\n";
        toml::from_str::<Manifest>(toml).unwrap_err();

        // The retired `id` is an unknown key like any other.
        let toml = "[[guest]]\nid = \"a\"\nsource.path = \"./a.wasm\"\n";
        toml::from_str::<Manifest>(toml).unwrap_err();

        let toml = "[[guest]]\nname = \"a\"\nsource.path = \"./a.wasm\"\nbogus = 1\n";
        toml::from_str::<Manifest>(toml).unwrap_err();

        let toml =
            "[[guest]]\nname = \"a\"\nsource.path = \"./a.wasm\"\n\n[transport]\nbogus = 1\n";
        toml::from_str::<Manifest>(toml).unwrap_err();
    }

    #[test]
    fn parse_guest_routes() {
        let toml = r#"
            [[guest]]
            name = "mcp"
            source.path = "./guests/mcp.wasm"
            routes.http = ["/mcp"]
            routes.messaging = ["specify.build.>"]
            routes.websocket = ["events.*"]
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.guests[0].routes.http, ["/mcp"]);
        assert_eq!(manifest.guests[0].routes.messaging, ["specify.build.>"]);
        assert_eq!(manifest.guests[0].routes.websocket, ["events.*"]);

        let routes = manifest.routes();
        assert_eq!(routes.http().resolve("/mcp/tool"), Some(&GuestId::from("mcp")));
        assert_eq!(routes.messaging().resolve("specify.build.x"), Some(&GuestId::from("mcp")));
        assert_eq!(routes.websocket().resolve("events.tick"), Some(&GuestId::from("mcp")));
    }

    #[test]
    fn routes_aggregate() {
        let toml = r#"
            [[guest]]
            name = "a"
            source.path = "./a.wasm"
            routes.http = ["/a"]

            [[guest]]
            name = "b"
            source.path = "./b.wasm"
            routes.http = ["/a/b"]
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        let routes = manifest.routes();
        // Longest-prefix matching is preserved across guest-owned lists.
        assert_eq!(routes.http().resolve("/a/x"), Some(&GuestId::from("a")));
        assert_eq!(routes.http().resolve("/a/b/x"), Some(&GuestId::from("b")));
    }

    #[test]
    fn reject_unknown_route_trigger() {
        let toml = "[[guest]]\nname = \"a\"\nsource.path = \"./a.wasm\"\nroutes.grpc = [\"/a\"]\n";
        toml::from_str::<Manifest>(toml).unwrap_err();
    }

    #[test]
    fn parse_and_resolve_mounts() {
        let toml = r#"
            [[guest]]
            name = "model"
            source.path = "./model.wasm"

            [[mount]]
            name = "."
            path = "../.."

            [[mount]]
            name = "shared"
            path = "/srv/shared"
            writable = true
        "#;

        let mut manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.mounts.len(), 2);
        assert_eq!(manifest.mounts[0].name, ".");
        assert!(!manifest.mounts[0].writable, "writable defaults to read-only");
        assert!(manifest.mounts[1].writable);

        let base = Path::new("/deploy/app");
        manifest.resolve_paths(base);
        let resolved = manifest.preopens();
        assert_eq!(resolved.len(), 2);
        // A relative path resolves against the manifest's directory; read-only by default.
        assert_eq!(resolved[0].name, ".");
        assert_eq!(resolved[0].host_path, base.join("../.."));
        assert!(!resolved[0].writable);
        // An absolute path passes through unchanged, and `writable` grants mutation.
        assert_eq!(resolved[1].host_path, PathBuf::from("/srv/shared"));
        assert!(resolved[1].writable);
    }

    #[test]
    fn parse_and_resolve_registries() {
        let toml = r#"
            [[guest]]
            name = "engine"
            source.path = "./engine.wasm"

            [registries]
            path = "wasm-pkg.toml"
        "#;

        let mut manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        manifest.resolve_paths(Path::new("/deploy/app"));
        // A relative path resolves against the manifest's directory, like a
        // guest source or a mount.
        assert_eq!(
            manifest.registries,
            Some(RegistryConfig::Path(PathBuf::from("/deploy/app/wasm-pkg.toml")))
        );
        #[cfg(feature = "loader")]
        manifest.validate(false).expect("a registries configuration is allowed");
        #[cfg(not(feature = "loader"))]
        {
            let error = manifest.validate(false).expect_err("registries need the loader feature");
            assert!(error.to_string().contains("without the `loader` feature"), "{error}");
        }
    }

    // A `[registries]` table names a path; the contents variant is the
    // macro's and the programmatic API's alone.
    #[test]
    fn reject_registries_contents_in_toml() {
        let toml = "[[guest]]\nname = \"a\"\nsource.path = \"./a.wasm\"\n\n\
             [registries]\ncontents = \"default_registry = 'ghcr.io'\"\n";
        toml::from_str::<Manifest>(toml).unwrap_err();
    }

    // The two carriers materialize the same way: a path is read, contents
    // are handed on as carried.
    #[test]
    fn registry_config_carriers() {
        let contents = "default_registry = \"ghcr.io\"\n";
        let inline = Manifest::new().registries(contents);
        assert_eq!(
            inline.registry_config().expect("contents materialize").as_deref(),
            Some(contents)
        );

        let path =
            std::env::temp_dir().join(format!("omnia_registries_{}.toml", std::process::id()));
        std::fs::write(&path, contents).expect("temp configuration should write");
        let file = Manifest::new().registries(path.clone());
        let read = file.registry_config();
        let _ = std::fs::remove_file(&path);
        assert_eq!(read.expect("path materializes").as_deref(), Some(contents));

        assert_eq!(Manifest::new().registry_config().expect("none is fine"), None);
    }

    #[test]
    fn cli_mount_relative() {
        let entry = Mount {
            name: ".".to_owned(),
            path: PathBuf::from("workspace"),
            writable: true,
        };
        // CLI mounts resolve against the process working directory, unlike
        // manifest mounts which resolve against the manifest's directory.
        let resolved = entry.resolve(Path::new("/cwd"));
        assert_eq!(resolved.host_path, PathBuf::from("/cwd/workspace"));
        assert!(resolved.writable);
    }

    #[test]
    fn defaults_to_in_process() {
        let toml = r#"
            [[guest]]
            name = "only"
            source.path = "./only.wasm"
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.transport.default, TransportKind::InProcess);
    }

    #[test]
    fn reject_non_default_transport() {
        let toml = "[[guest]]\nname = \"only\"\nsource.path = \"./only.wasm\"\n\n\
             [transport]\ndefault = \"unix\"\n";
        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert!(manifest.validate(false).is_err(), "distributed transport is not yet implemented");
    }

    #[test]
    fn parse_file() {
        let path =
            std::env::temp_dir().join(format!("omnia_manifest_ok_{}.toml", std::process::id()));
        std::fs::write(&path, "[[guest]]\nsource.path = \"./only.wasm\"\n")
            .expect("temp manifest should write");

        let manifest = Manifest::load(&path).expect("manifest should load");
        let _ = std::fs::remove_file(&path);

        assert_eq!(manifest.guests.len(), 1);
        assert_eq!(manifest.guests[0].name, "only", "the file's stem names the guest");
        let SourceSpec::Path(source) = &manifest.guests[0].source else {
            panic!("expected path source");
        };
        assert!(source.is_absolute());
    }

    #[test]
    fn parse_command_flag() {
        let toml = "[[guest]]\nname = \"helper\"\nsource.path = \"./helper.wasm\"\n\n\
             [[guest]]\nname = \"app\"\nsource.path = \"./app.wasm\"\ncommand = true\n";
        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        manifest.validate(false).expect("one marked guest validates");
        assert!(!manifest.guests[0].command, "the flag defaults to false");
        assert_eq!(manifest.command_guest(), Some(GuestId::from("app")));
    }

    #[test]
    fn reject_multiple_command_guests() {
        let manifest = Manifest::new()
            .guest(GuestEntry::new("a", "./a.wasm").command())
            .guest(GuestEntry::new("b", "./b.wasm").command());
        let error = manifest.validate(false).expect_err("two marked guests must be rejected");
        assert!(error.to_string().contains("at most one guest may be the command guest"));
    }

    #[test]
    fn reject_duplicate_guest_names() {
        let toml = "[[guest]]\nname = \"same\"\nsource.path = \"./a.wasm\"\n\n\
             [[guest]]\nname = \"same\"\nsource.path = \"./b.wasm\"\n";
        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        let error = manifest.validate(false).expect_err("duplicate guest names must be rejected");
        assert!(error.to_string().contains("duplicate [[guest]] name `same`"), "{error}");
    }

    // Two entries reading files of the same stem collide on the derived
    // name, and a `name` on one of them resolves it.
    #[test]
    fn name_from_stem() {
        let toml = "[[guest]]\nsource.path = \"./a/echo.wasm\"\n\n\
             [[guest]]\nsource.path = \"./b/echo.wasm\"\n";
        let mut manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.guests[0].name, "", "the parser derives nothing");
        manifest.resolve_names();
        assert_eq!(manifest.guests[0].name, "echo");
        assert_eq!(manifest.guests[1].name, "echo");
        let error = manifest.validate(false).expect_err("two `echo` guests must be rejected");
        assert!(error.to_string().contains("duplicate [[guest]] name `echo`"), "{error}");

        let toml = "[[guest]]\nsource.path = \"./a/echo.wasm\"\n\n\
             [[guest]]\nname = \"other\"\nsource.path = \"./b/echo.wasm\"\n";
        let mut manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        manifest.resolve_names();
        manifest.validate(false).expect("distinct names validate");
        assert_eq!(manifest.command_guest(), None);
        assert_eq!(manifest.guests[1].name, "other", "a given name is kept");
    }

    // Only a path derives a name; an entry nothing names is refused, not
    // registered under the empty identity.
    #[test]
    fn reject_unnamed_guest() {
        let toml = "[[guest]]\nsource.oci = \"ghcr.io/acme/echo@sha256:00\"\n";
        let mut manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        manifest.resolve_names();
        let error = manifest.validate(false).expect_err("an unnamed guest must be rejected");
        assert!(error.to_string().contains("names no guest"), "{error}");

        let error = Manifest::new()
            .guest(GuestEntry::new("", b"\0asm"))
            .validate(false)
            .expect_err("an unnamed embedded guest must be rejected");
        assert!(error.to_string().contains("names no guest"), "{error}");
    }

    #[test]
    fn embedded_named_by_stem() {
        let entry = GuestEntry::embedded(
            "/build/target/wasm32-wasip2/release/examples/engine.wasm",
            b"\0asm",
        );
        assert_eq!(entry.name, "engine");
        assert!(matches!(entry.source, SourceSpec::Bytes(_)));
    }

    #[test]
    fn reject_without_guests() {
        let manifest: Manifest =
            toml::from_str("[transport]\ndefault = \"unix\"\n").expect("manifest should parse");
        assert!(
            manifest.validate(false).is_err(),
            "a static manifest with no guests must be rejected"
        );
        assert!(
            Manifest::new().validate(true).is_ok(),
            "a dynamic deployment may start with no guests"
        );
    }

    #[test]
    fn bytes_source() {
        // `b"..."` is `&'static [u8; N]` — the `include_bytes!` shape.
        let manifest = Manifest::new()
            .guest(GuestEntry::new("baked", b"\0asm"))
            .guest(GuestEntry::new("read", Vec::from(*b"\0asm")));

        assert!(matches!(manifest.guests[0].source, SourceSpec::Bytes(_)));
        assert_eq!(format!("{:?}", manifest.guests[0].source), "Bytes(4 bytes)");

        let sources = manifest.sources().expect("bytes sources resolve");
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].id(), &GuestId::from("baked"));
        assert_eq!(sources[1].id(), &GuestId::from("read"));
    }

    #[test]
    fn build_programmatically() {
        let manifest = Manifest::new()
            .guest(
                GuestEntry::new("router", "router.wasm")
                    .route_http("/router")
                    .route_messaging("jobs.>"),
            )
            .guest(GuestEntry::new("responder", "responder.wasm").route_websocket("events.*"))
            .mounts([Mount {
                name: ".".to_owned(),
                path: PathBuf::from("workspace"),
                writable: true,
            }]);

        manifest.validate(false).expect("manifest should validate");
        assert_eq!(manifest.guests.len(), 2);
        assert_eq!(manifest.mounts.len(), 1);
        assert_eq!(manifest.guests[0].routes.http, ["/router"]);
        assert_eq!(manifest.guests[0].routes.messaging, ["jobs.>"]);
        assert_eq!(manifest.guests[1].routes.websocket, ["events.*"]);
    }
}
