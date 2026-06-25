//! # Deployment manifest (`omni.toml`)
//!
//! Registry population, routing, linking, and transport are *deployment*
//! decisions, not build-time ones. They live in a startup manifest loaded
//! before the registry is built. The manifest is optional and sparse: any field
//! left out falls back to a synthesized default, and with no file at all Omnia
//! runs the single-guest zero-config default.
//!
//! The manifest is parsed **generically** — Omnia sees opaque [`GuestId`]s and
//! interface *strings*, never `source:`/`target:`/`mcp`. Consumers write the
//! concrete file; the floor stays domain-agnostic.
//!
//! Phase 1 consumes the `[[guest]]` population (file sources) and parses the
//! `[transport]` section; Phase 1b adds the `[[route.*]]` tables. `link`
//! allow-lists are accepted (so a richer file still loads) but wired into the
//! shared linker in a later phase.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// The deployment manifest: which guests load and how host-mediated calls
/// travel.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// Registry population: each entry maps an identity to a source.
    #[serde(rename = "guest")]
    pub guests: Vec<GuestEntry>,
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
        Ok(())
    }
}

/// A single registry population entry.
#[derive(Clone, Debug, Deserialize)]
pub struct GuestEntry {
    /// Opaque guest identity (the floor never parses it).
    pub id: String,
    /// Where the guest's component bytes come from.
    pub source: SourceSpec,
    /// Imports the host should dispatch (host-mediated). Parsed in Phase 1;
    /// wired into the shared linker in a later phase.
    #[serde(default)]
    pub link: Vec<String>,
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

/// A single HTTP route: a path prefix mapped to a target guest.
#[derive(Clone, Debug, Deserialize)]
pub struct HttpRoute {
    /// The path prefix; the longest matching prefix wins.
    pub prefix: String,
    /// The target guest identity (opaque to the floor).
    pub guest: String,
}

/// A single topic/route entry: a NATS-style pattern mapped to a target guest.
/// Messaging spells the pattern `topic`; websocket spells it `route`.
#[derive(Clone, Debug, Deserialize)]
pub struct TopicRoute {
    /// The match pattern (`.`-tokenised, `*` one token, `>` trailing tokens).
    #[serde(alias = "route")]
    pub topic: String,
    /// The target guest identity (opaque to the floor).
    pub guest: String,
}

/// Where a guest's component bytes come from.
///
/// Modelled as an externally tagged enum so TOML's `source.path = "..."`,
/// `source.embedded = "..."`, and `source.oci = "..."` each select a variant.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceSpec {
    /// A local `.wasm` / pre-compiled `.bin` path (resolved relative to the
    /// manifest's directory).
    Path(PathBuf),
    /// A build-time `include_bytes!` blob, by name. Accepted by the parser;
    /// resolving it lands with the embedded source phase.
    Embedded(String),
    /// A digest-pinned OCI reference. Accepted by the parser; the puller lands
    /// as a follow-up.
    Oci(String),
}

/// Transport configuration. Parsed and validated in Phase 1; the dispatch path
/// that consumes it lands with host-mediated linking.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Transport {
    /// The default transport when no per-target override applies.
    pub default: TransportKind,
    /// Per-target transport overrides (identity -> transport).
    #[serde(rename = "target")]
    pub targets: HashMap<String, TransportOverride>,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            default: TransportKind::InProcess,
            targets: HashMap::new(),
        }
    }
}

/// A transport mechanism for host-mediated calls.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    /// In-process byte pipe — the co-located default.
    #[default]
    InProcess,
    /// Unix-domain socket (same node, separate processes).
    Unix,
    /// NATS (cross-node).
    Nats,
    /// QUIC (cross-node).
    Quic,
}

/// A per-target transport override for distributed nodes.
#[derive(Clone, Debug, Deserialize)]
pub struct TransportOverride {
    /// The transport mechanism.
    pub kind: TransportKind,
    /// The transport address, when the mechanism needs one.
    #[serde(default)]
    pub address: Option<String>,
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
    fn defaults_to_in_process() {
        let toml = r#"
            [[guest]]
            id = "only"
            source.path = "./only.wasm"
        "#;

        let manifest: Manifest = toml::from_str(toml).expect("manifest should parse");
        assert_eq!(manifest.transport.default, TransportKind::InProcess);
        assert!(manifest.transport.targets.is_empty());
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
