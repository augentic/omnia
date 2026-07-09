//! # Deployment manifest (`omnia.toml`)
//!
//! Registry population, routing, linking, and transport are *deployment*
//! decisions, not build-time ones. They live in a startup manifest loaded
//! before the registry is built. The manifest is optional and sparse: any field
//! left out falls back to a synthesized default, and with no file at all Omnia
//! runs the single-guest zero-config default.
//!
//! The manifest is parsed **generically** — Omnia sees opaque [`GuestId`]s and
//! interface *strings*, never `source:`/`target:`/`mcp`. Consumers write the
//! concrete file; the runtime core stays domain-agnostic.
//!
//! The `[[guest]]` population (file sources), the `[[route.*]]` tables, and the
//! per-guest `link` allow-lists (which drive host-mediated dynamic linking) are
//! all consumed. Distributed `[transport]` is not yet implemented: only the
//! in-process default is accepted, and any other value is rejected at load.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::source::Source;
use crate::mount::ResolvedPreopen;
use crate::registry::{CliRoutes, GuestId, HttpRoutes, PatternRoutes, Routes};

/// The deployment manifest: which guests load and how host-mediated calls
/// travel.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// Registry population: each entry maps an identity to a source.
    #[serde(rename = "guest")]
    pub guests: Vec<GuestEntry>,
    /// Working-tree mounts preopened into the guest sandbox.
    #[serde(rename = "mount")]
    pub mounts: Vec<Mount>,
    /// Inbound route tables, one list per trigger.
    pub route: RouteSpec,
    /// Transport configuration for host-mediated calls.
    pub transport: Transport,
}

impl Manifest {
    /// Load and parse a manifest from `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, cannot be parsed as TOML, or
    /// defines no `[[guest]]` entries.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        let manifest: Self = toml::from_str(&text)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate manifest-level invariants surfaced before the registry is built.
    fn validate(&self) -> Result<()> {
        if self.guests.is_empty() {
            bail!("manifest defines no [[guest]] entries");
        }
        let mut ids = BTreeSet::new();
        for entry in &self.guests {
            if !ids.insert(entry.id.as_str()) {
                bail!("duplicate [[guest]] id `{}`: guest identities must be unique", entry.id);
            }
        }
        if self.transport.default != TransportKind::InProcess {
            bail!(
                "transport `{:?}` is not yet implemented; only in-process transport is supported",
                self.transport.default
            );
        }
        Ok(())
    }

    /// Telemetry/component name for this deployment.
    ///
    /// The first `[[guest]]` entry doubles as the name for now.
    #[must_use]
    pub fn name(&self) -> &str {
        self.guests.first().map_or("omnia", |entry| entry.id.as_str())
    }

    /// Resolve every `[[guest]]` source into a loadable [`Source`].
    ///
    /// Paths in the manifest are resolved relative to `base` (typically the
    /// manifest's parent directory).
    ///
    /// # Errors
    ///
    /// Returns an error if a guest uses a source kind not yet supported.
    pub fn sources(&self, base: &Path) -> Result<Vec<Source>> {
        let mut sources = Vec::with_capacity(self.guests.len());
        for entry in &self.guests {
            let id = GuestId::from(entry.id.as_str());
            match &entry.source {
                SourceSpec::Path(path) => {
                    let resolved = if path.is_absolute() { path.clone() } else { base.join(path) };
                    sources.push(Source::with_id(id, resolved));
                }
                SourceSpec::Oci(reference) => {
                    bail!("guest `{id}`: OCI source `{reference}` is not yet supported")
                }
            }
        }
        Ok(sources)
    }

    /// Union of the per-guest `link` allow-lists — the host-mediated interfaces.
    ///
    /// The linker is shared, so an interface dispatched for one guest is wired
    /// once for all.
    #[must_use]
    pub fn links(&self) -> BTreeSet<Box<str>> {
        self.guests
            .iter()
            .flat_map(|entry| entry.link.iter())
            .map(|interface| Box::from(interface.as_str()))
            .collect()
    }

    /// Per-trigger route tables parsed from the manifest's `[[route.*]]` sections.
    #[must_use]
    pub fn routes(&self) -> Routes {
        self.route.to_routes()
    }

    /// Resolve every `[[mount]]` into a [`ResolvedPreopen`].
    ///
    /// Host paths resolve relative to `base` exactly as `[[guest]]` sources do,
    /// and `writable` selects read-only (review) versus read+write (edit) WASI
    /// permissions.
    #[must_use]
    pub fn mounts(&self, base: &Path) -> Vec<ResolvedPreopen> {
        self.mounts.iter().map(|entry| entry.resolve(base)).collect()
    }
}

/// A single workspace mount: a host directory preopened into the guest
/// sandbox under a guest-visible name.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Mount {
    /// Guest-visible name `preopens.get-directories()` returns (e.g. `.`).
    pub name: String,
    /// Host path (absolute, or relative to the manifest's directory).
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

impl FromStr for Mount {
    type Err = anyhow::Error;

    /// Parse a CLI `--mount` spec: comma-separated `path=<host-path>`,
    /// `name=<guest-name>`, and a bare `writable` (or `writable=<bool>`) flag. A
    /// lone token without `=` is taken as the path, so `workspace` and
    /// `workspace,writable` are shorthands; `name` defaults to `.` and the mount
    /// is read-only unless `writable` is present.
    fn from_str(spec: &str) -> Result<Self> {
        let mut path: Option<PathBuf> = None;
        let mut name: Option<String> = None;
        let mut writable = false;

        for token in spec.split(',').map(str::trim).filter(|token| !token.is_empty()) {
            match token.split_once('=') {
                Some(("path", value)) => path = Some(PathBuf::from(value)),
                Some(("name", value)) => name = Some(value.to_owned()),
                Some(("writable", value)) => {
                    writable = value.parse().with_context(|| {
                        format!("mount `writable` expects a bool, got `{value}`")
                    })?;
                }
                Some((key, _)) => bail!("unknown mount key `{key}` in `--mount {spec}`"),
                None if token == "writable" => writable = true,
                None => {
                    if path.replace(PathBuf::from(token)).is_some() {
                        bail!("mount `--mount {spec}` sets the path more than once");
                    }
                }
            }
        }

        let path =
            path.with_context(|| format!("mount `--mount {spec}` is missing `path=<host-path>`"))?;
        Ok(Self {
            name: name.unwrap_or_else(|| ".".to_owned()),
            path,
            writable,
        })
    }
}

/// A single registry population entry.
#[derive(Clone, Debug, Deserialize)]
pub struct GuestEntry {
    /// Opaque guest identity (the runtime core never parses it).
    pub id: String,
    /// Where the guest's component bytes come from.
    pub source: SourceSpec,
    /// Interfaces the host dispatches on this guest's behalf (host-mediated
    /// dynamic linking); the runtime core polyfills each on the shared linker.
    #[serde(default)]
    pub link: Vec<String>,
}

/// Where a guest's component bytes come from.
///
/// Modelled as an externally tagged enum so TOML's `source.path = "..."` and
/// `source.oci = "..."` each select a variant.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceSpec {
    /// A local `.wasm` / pre-compiled `.bin` path (resolved relative to the
    /// manifest's directory).
    Path(PathBuf),
    /// A digest-pinned OCI reference. Accepted by the parser and surfaced in the
    /// "not yet supported" error; the puller that consumes it lands as a
    /// follow-up.
    Oci(String),
}

/// Inbound routing: one list of routes per trigger, orthogonal to population
/// (a guest may carry no route).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RouteSpec {
    /// HTTP routes, matched by longest path prefix.
    pub http: Vec<HttpRoute>,
    /// Messaging routes, matched by NATS-style topic pattern.
    pub messaging: Vec<TopicRoute>,
    /// WebSocket routes, matched by NATS-style route pattern.
    pub websocket: Vec<TopicRoute>,
}

impl RouteSpec {
    /// Convert the manifest's parsed routes into the registry's `GuestId`-typed,
    /// per-trigger route tables.
    #[must_use]
    pub fn to_routes(&self) -> Routes {
        let http = HttpRoutes::new(
            self.http.iter().map(|e| (e.prefix.clone(), GuestId::from(e.guest.as_str()))),
        );
        let messaging = PatternRoutes::new(
            self.messaging.iter().map(|e| (e.topic.clone(), GuestId::from(e.guest.as_str()))),
        );
        let websocket = PatternRoutes::new(
            self.websocket.iter().map(|e| (e.topic.clone(), GuestId::from(e.guest.as_str()))),
        );
        // `[[route.cli]]` is not yet parsed; an empty table makes a sole
        // `wasi:cli/run` exporter the catch-all (multi-command routing is
        // deferred).
        Routes::new(http, messaging, websocket, CliRoutes::default())
    }
}

/// A single HTTP route: a path prefix mapped to a target guest.
#[derive(Clone, Debug, Deserialize)]
pub struct HttpRoute {
    /// The path prefix; the longest matching prefix wins.
    pub prefix: String,
    /// The target guest identity (opaque to the runtime core).
    pub guest: String,
}

/// A single topic/route entry: a NATS-style pattern mapped to a target guest.
/// Messaging spells the pattern `topic`; websocket spells it `route`.
#[derive(Clone, Debug, Deserialize)]
pub struct TopicRoute {
    /// The match pattern (`.`-tokenised, `*` one token, `>` trailing tokens).
    #[serde(alias = "route")]
    pub topic: String,
    /// The target guest identity (opaque to the runtime core).
    pub guest: String,
}

/// Transport configuration for host-mediated calls.
///
/// Only the in-process default is implemented; [`Manifest::validate`] rejects any
/// other value, and `#[serde(deny_unknown_fields)]` turns a stale distributed
/// `[transport.target.*]` section into a loud parse error rather than a silent
/// no-op.
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
    /// In-process byte pipe — the co-located default (the only implemented kind).
    #[default]
    InProcess,
    /// Unix-domain socket (same node, separate processes).
    Unix,
    /// NATS (cross-node).
    Nats,
    /// QUIC (cross-node).
    Quic,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multi_guest() {
        let toml = r#"
            [[guest]]
            id = "workflow"
            source.path = "./guests/workflow.wasm"
            link = ["augentic:specify/source", "augentic:specify/target"]

            [[guest]]
            id = "mcp"
            source.path = "./guests/mcp.wasm"

            [transport]
            default = "in-process"
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.guests.len(), 2);
        assert_eq!(manifest.guests[0].id, "workflow");
        assert_eq!(manifest.guests[0].link.len(), 2);
        assert!(matches!(manifest.guests[1].source, SourceSpec::Path(_)));
        assert_eq!(manifest.transport.default, TransportKind::InProcess);
    }

    #[test]
    fn parse_route_tables() {
        let toml = r#"
            [[guest]]
            id = "mcp"
            source.path = "./guests/mcp.wasm"

            [[route.http]]
            prefix = "/mcp"
            guest = "mcp"

            [[route.messaging]]
            topic = "specify.build.>"
            guest = "mcp"

            [[route.websocket]]
            route = "events.*"
            guest = "mcp"
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.route.http.len(), 1);
        assert_eq!(manifest.route.http[0].prefix, "/mcp");
        assert_eq!(manifest.route.http[0].guest, "mcp");
        assert_eq!(manifest.route.messaging[0].topic, "specify.build.>");
        // The websocket trigger reuses the topic entry via its `route` alias.
        assert_eq!(manifest.route.websocket[0].topic, "events.*");
    }

    #[test]
    fn parse_and_resolve_mounts() {
        let toml = r#"
            [[guest]]
            id = "model"
            source.path = "./model.wasm"

            [[mount]]
            name = "."
            path = "../.."

            [[mount]]
            name = "shared"
            path = "/srv/shared"
            writable = true
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.mounts.len(), 2);
        assert_eq!(manifest.mounts[0].name, ".");
        assert!(!manifest.mounts[0].writable, "writable defaults to read-only");
        assert!(manifest.mounts[1].writable);

        let base = Path::new("/deploy/app");
        let resolved = manifest.mounts(base);
        assert_eq!(resolved.len(), 2);
        // A relative path resolves against the manifest's directory; read-only by default.
        assert_eq!(resolved[0].name, ".");
        assert_eq!(resolved[0].host_path, base.join("../.."));
        assert!(!resolved[0].dir_perms.contains(wasmtime_wasi::DirPerms::MUTATE));
        // An absolute path passes through unchanged, and `writable` grants mutation.
        assert_eq!(resolved[1].host_path, PathBuf::from("/srv/shared"));
        assert!(resolved[1].dir_perms.contains(wasmtime_wasi::DirPerms::MUTATE));
    }

    #[test]
    fn parse_mount_full_spec() {
        let entry: Mount = "path=workspace,name=.,writable".parse().expect("spec parses");
        assert_eq!(entry.path, PathBuf::from("workspace"));
        assert_eq!(entry.name, ".");
        assert!(entry.writable);
    }

    #[test]
    fn parse_mount_bare_path_shorthand() {
        let entry: Mount = "workspace".parse().expect("bare path parses");
        assert_eq!(entry.path, PathBuf::from("workspace"));
        assert_eq!(entry.name, ".", "name defaults to `.`");
        assert!(!entry.writable, "a mount is read-only unless `writable` is given");
    }

    #[test]
    fn parse_mount_bare_writable_shorthand() {
        let entry: Mount = "workspace,writable".parse().expect("shorthand parses");
        assert_eq!(entry.path, PathBuf::from("workspace"));
        assert!(entry.writable);
    }

    #[test]
    fn parse_mount_requires_path() {
        assert!("name=.,writable".parse::<Mount>().is_err(), "a mount must name a path");
    }

    #[test]
    fn parse_mount_rejects_unknown_key() {
        assert!("path=x,bogus=1".parse::<Mount>().is_err(), "unknown keys are rejected");
    }

    #[test]
    fn cli_mount_resolves_relative_to_base() {
        let entry: Mount = "path=workspace,writable".parse().expect("spec parses");
        // CLI mounts resolve against the process working directory, unlike
        // manifest mounts which resolve against the manifest's directory.
        let resolved = entry.resolve(Path::new("/cwd"));
        assert_eq!(resolved.host_path, PathBuf::from("/cwd/workspace"));
        assert!(resolved.dir_perms.contains(wasmtime_wasi::DirPerms::MUTATE));
    }

    #[test]
    fn defaults_to_in_process() {
        let toml = r#"
            [[guest]]
            id = "only"
            source.path = "./only.wasm"
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.transport.default, TransportKind::InProcess);
    }

    #[test]
    fn reject_non_default_transport() {
        let path = std::env::temp_dir()
            .join(format!("omnia_manifest_transport_{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[guest]]\nid = \"only\"\nsource.path = \"./only.wasm\"\n\n\
             [transport]\ndefault = \"unix\"\n",
        )
        .expect("temp manifest should write");

        let result = Manifest::load(&path);
        let _ = std::fs::remove_file(&path);

        assert!(result.is_err(), "distributed transport is not yet implemented");
    }

    #[test]
    fn reject_stale_target_section() {
        // A leftover distributed-transport target must fail loudly, not be ignored.
        let toml = "[[guest]]\nid = \"only\"\nsource.path = \"./only.wasm\"\n\n\
             [transport.target.remote]\nkind = \"unix\"\n";
        toml::from_str::<Manifest>(toml).unwrap_err();
    }

    #[test]
    fn parse_file() {
        let path =
            std::env::temp_dir().join(format!("omnia_manifest_ok_{}.toml", std::process::id()));
        std::fs::write(&path, "[[guest]]\nid = \"only\"\nsource.path = \"./only.wasm\"\n")
            .expect("temp manifest should write");

        let manifest = Manifest::load(&path).expect("manifest should load");
        let _ = std::fs::remove_file(&path);

        assert_eq!(manifest.guests.len(), 1);
        assert_eq!(manifest.guests[0].id, "only");
    }

    #[test]
    fn reject_duplicate_guest_ids() {
        let path =
            std::env::temp_dir().join(format!("omnia_manifest_dup_{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[guest]]\nid = \"same\"\nsource.path = \"./a.wasm\"\n\n\
             [[guest]]\nid = \"same\"\nsource.path = \"./b.wasm\"\n",
        )
        .expect("temp manifest should write");

        let result = Manifest::load(&path);
        let _ = std::fs::remove_file(&path);

        let error = result.expect_err("duplicate guest ids must be rejected");
        assert!(error.to_string().contains("duplicate [[guest]] id `same`"), "{error}");
    }

    #[test]
    fn reject_without_guests() {
        let path =
            std::env::temp_dir().join(format!("omnia_manifest_empty_{}.toml", std::process::id()));
        std::fs::write(&path, "[transport]\ndefault = \"unix\"\n")
            .expect("temp manifest should write");

        let result = Manifest::load(&path);
        let _ = std::fs::remove_file(&path);

        assert!(result.is_err(), "a manifest with no guests must be rejected");
    }
}
