//! # Trigger routing
//!
//! Maps an inbound trigger to a guest [`GuestId`] generically: the floor routes
//! on opaque identities and route strings only, never on domain concepts.
//!
//! Two table shapes cover the triggers:
//! - [`HttpRoutes`] — longest-prefix match on a request path.
//! - [`TopicRoutes`] — NATS-style token match on a messaging topic (also
//!   used for websocket routes via the manifest's `route` alias).
//!
//! [`Router`] layers the capability-based default routing of the guest-registry
//! design over a table: with no routes configured a sole handler exporter is the
//! catch-all for its trigger, zero exporters is inert, and two or more exporters
//! require explicit routes to disambiguate.

use std::collections::HashMap;

use anyhow::{Result, bail};
use wasmtime::component::InstancePre;

use crate::registry::{GuestId, Registry};

/// A per-trigger route table resolving a routing key to a target identity.
pub trait Resolver {
    /// Resolve a routing key (a path, topic, ...) to a target guest identity.
    fn resolve(&self, key: &str) -> Option<&GuestId>;

    /// Iterate the identities every route in the table targets.
    fn targets(&self) -> impl Iterator<Item = &GuestId>;

    /// Returns `true` when the table holds no routes.
    fn is_empty(&self) -> bool;
}

/// Longest-prefix HTTP route table: `/target/omnia` wins over `/target`.
#[derive(Clone, Debug, Default)]
pub struct HttpRoutes {
    /// `(prefix, target)` pairs, sorted by prefix length descending so the
    /// first match is the longest.
    entries: Vec<(String, GuestId)>,
}

impl HttpRoutes {
    /// Build a table from `(prefix, target)` pairs, ordering longest prefix
    /// first so resolution is a simple find.
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = (String, GuestId)>) -> Self {
        let mut entries: Vec<(String, GuestId)> = entries.into_iter().collect();
        entries.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
        Self { entries }
    }
}

impl Resolver for HttpRoutes {
    fn resolve(&self, key: &str) -> Option<&GuestId> {
        self.entries.iter().find(|(prefix, _)| path_has_prefix(key, prefix)).map(|(_, id)| id)
    }

    fn targets(&self) -> impl Iterator<Item = &GuestId> {
        self.entries.iter().map(|(_, id)| id)
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// NATS-style topic route table: `.`-tokenised, `*` matches exactly one token,
/// `>` matches one or more trailing tokens. Drives messaging (`topic`) and
/// websocket (`route`).
#[derive(Clone, Debug, Default)]
pub struct TopicRoutes {
    /// `(pattern, target)` pairs; the first match in declaration order wins.
    entries: Vec<(String, GuestId)>,
}

impl TopicRoutes {
    /// Build a table from `(pattern, target)` pairs, preserving declaration
    /// order (first match wins).
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = (String, GuestId)>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }
}

impl Resolver for TopicRoutes {
    fn resolve(&self, key: &str) -> Option<&GuestId> {
        self.entries.iter().find(|(pattern, _)| topic_matches(key, pattern)).map(|(_, id)| id)
    }

    fn targets(&self) -> impl Iterator<Item = &GuestId> {
        self.entries.iter().map(|(_, id)| id)
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The per-trigger route tables a registry carries, parsed from the manifest's
/// `[[route.*]]` sections.
#[derive(Clone, Debug, Default)]
pub struct Routes {
    http: HttpRoutes,
    messaging: TopicRoutes,
    websocket: TopicRoutes,
}

impl Routes {
    /// Assemble the per-trigger tables.
    #[must_use]
    pub const fn new(http: HttpRoutes, messaging: TopicRoutes, websocket: TopicRoutes) -> Self {
        Self {
            http,
            messaging,
            websocket,
        }
    }

    /// The HTTP (longest-prefix) route table.
    #[must_use]
    pub const fn http(&self) -> &HttpRoutes {
        &self.http
    }

    /// The messaging (topic) route table.
    #[must_use]
    pub const fn messaging(&self) -> &TopicRoutes {
        &self.messaging
    }

    /// The websocket (route) route table.
    #[must_use]
    pub const fn websocket(&self) -> &TopicRoutes {
        &self.websocket
    }

    /// Iterate every identity any route targets across all triggers — used to
    /// validate routes name registered guests.
    pub fn targets(&self) -> impl Iterator<Item = &GuestId> {
        self.http.targets().chain(self.messaging.targets()).chain(self.websocket.targets())
    }
}

/// Capability-based routing for one trigger, layered over a route table.
#[derive(Clone, Debug)]
pub enum Router<R> {
    /// Explicit routes drive the trigger; an unmatched key is a miss.
    Routed(R),
    /// A sole handler exporter catches the whole trigger; every call fans into
    /// it regardless of the routing key.
    CatchAll(GuestId),
    /// No guest answers this trigger.
    Inert,
}

impl<R: Resolver> Router<R> {
    /// Decide how `trigger` routes, given the guests that export its handler
    /// (`capable`, in a stable order) and the configured `table`.
    ///
    /// With routes configured the trigger is fully route-driven, and every
    /// target must be capable. Otherwise routing defaults by exporter count: one
    /// is the catch-all, none is inert, and two or more is ambiguous.
    ///
    /// # Errors
    ///
    /// Returns an error if a route targets a guest that does not export the
    /// handler, or if two or more guests export it with no routes to
    /// disambiguate.
    pub fn build(trigger: &str, capable: &[GuestId], resolver: R) -> Result<Self> {
        if !resolver.is_empty() {
            for target in resolver.targets() {
                if !capable.contains(target) {
                    bail!(
                        "route for trigger `{trigger}` names `{target}`, which does not export \
                         the `{trigger}` handler"
                    );
                }
            }
            return Ok(Self::Routed(resolver));
        }

        match capable {
            [] => Ok(Self::Inert),
            [only] => Ok(Self::CatchAll(only.clone())),
            many => {
                let names = many.iter().map(GuestId::as_str).collect::<Vec<_>>().join(", ");
                bail!(
                    "trigger `{trigger}` has {} capable guests ({names}) but no routes",
                    many.len()
                )
            }
        }
    }

    /// Resolve a routing `key` to a target identity, or `None` on a miss or an
    /// inert trigger. A catch-all ignores the key.
    #[must_use]
    pub fn resolve(&self, key: &str) -> Option<&GuestId> {
        match self {
            Self::Routed(resolver) => resolver.resolve(key),
            Self::CatchAll(id) => Some(id),
            Self::Inert => None,
        }
    }

    /// The catch-all target, if this trigger fans an unkeyed call into a sole
    /// exporter (used by websocket events that carry no route).
    #[must_use]
    pub const fn catch_all(&self) -> Option<&GuestId> {
        match self {
            Self::CatchAll(id) => Some(id),
            Self::Routed(_) | Self::Inert => None,
        }
    }

    /// Returns `true` when no guest answers this trigger.
    #[must_use]
    pub const fn is_inert(&self) -> bool {
        matches!(self, Self::Inert)
    }
}

/// Pairs a per-trigger [`Router`] with the typed binding indices of every
/// capable guest.
///
/// A trigger server builds this once, then resolves a routing key straight to
/// the indices it needs to instantiate. `I` is the handler-specific generated
/// index type (e.g. `ServiceIndices` for HTTP); the floor never names it — it
/// only stores it and hands it back.
pub struct TriggerRouter<I, R> {
    indices: HashMap<GuestId, I>,
    router: Router<R>,
}

impl<I, R: Resolver> TriggerRouter<I, R> {
    /// Probe every registered guest for the trigger's handler — a guest is
    /// *capable* exactly when `probe` succeeds — then build the [`Router`] over
    /// the capable set and the configured route `table`.
    ///
    /// # Errors
    ///
    /// Returns an error if [`Router::build`] rejects the capable set and table:
    /// a route names a guest that does not export the handler, or two or more
    /// guests export it with no routes to disambiguate.
    pub fn build<T, E, F>(
        registry: &Registry<T>, trigger: &str, table: R, mut probe: F,
    ) -> Result<Self>
    where
        F: FnMut(&InstancePre<T>) -> Result<I, E>,
    {
        let mut indices = HashMap::new();
        let mut capable = Vec::new();
        for guest in registry.guests() {
            if let Ok(index) = probe(guest.instance_pre()) {
                capable.push(guest.id().clone());
                indices.insert(guest.id().clone(), index);
            }
        }
        let router = Router::build(trigger, &capable, table)?;
        Ok(Self { indices, router })
    }

    /// Returns `true` when no guest answers this trigger.
    #[must_use]
    pub const fn is_inert(&self) -> bool {
        self.router.is_inert()
    }

    /// Resolve a routing `key` to the target identity and its binding indices,
    /// or `None` on a miss or an inert trigger. A catch-all ignores the key.
    #[must_use]
    pub fn resolve(&self, key: &str) -> Option<(&GuestId, &I)> {
        let id = self.router.resolve(key)?;
        self.indices.get(id).map(|index| (id, index))
    }

    /// The sole-exporter catch-all target and its binding indices, if this
    /// trigger fans an unkeyed call into a single exporter (used by websocket
    /// events that carry no route).
    #[must_use]
    pub fn catch_all(&self) -> Option<(&GuestId, &I)> {
        let id = self.router.catch_all()?;
        self.indices.get(id).map(|index| (id, index))
    }
}

/// Path-prefix match that respects segment boundaries: `/a` matches `/a` and
/// `/a/b` but not `/ab`.
fn path_has_prefix(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || prefix.ends_with('/'))
}

/// NATS-style topic match: `.`-tokenised, `*` matches exactly one token, and `>`
/// matches one or more trailing tokens (and must be the final pattern token).
fn topic_matches(topic: &str, pattern: &str) -> bool {
    // Walk both token streams in lockstep without materialising either: `*`
    // consumes exactly one topic token, a literal must match it, and `>` (which
    // must be the final pattern token) swallows one or more trailing tokens.
    let mut topic_tokens = topic.split('.');
    let mut pattern_tokens = pattern.split('.').peekable();

    while let Some(token) = pattern_tokens.next() {
        match token {
            ">" => return pattern_tokens.peek().is_none() && topic_tokens.next().is_some(),
            "*" => {
                if topic_tokens.next().is_none() {
                    return false;
                }
            }
            literal => {
                if topic_tokens.next() != Some(literal) {
                    return false;
                }
            }
        }
    }

    // Every pattern token matched; it is a full match only if the topic is also
    // exhausted (equal token counts).
    topic_tokens.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> GuestId {
        GuestId::from(value)
    }

    #[test]
    fn http_longest_prefix_() {
        let routes =
            HttpRoutes::new([("/a".to_owned(), id("short")), ("/a/b".to_owned(), id("long"))]);
        assert_eq!(routes.resolve("/a/b/c"), Some(&id("long")));
        assert_eq!(routes.resolve("/a/x"), Some(&id("short")));
        assert_eq!(routes.resolve("/other"), None);
    }

    #[test]
    fn http_prefix_segments() {
        let routes = HttpRoutes::new([("/a".to_owned(), id("a"))]);
        assert_eq!(routes.resolve("/a"), Some(&id("a")));
        assert_eq!(routes.resolve("/a/deep"), Some(&id("a")));
        assert_eq!(routes.resolve("/abc"), None);
    }

    #[test]
    fn topic_wildcard() {
        let routes = TopicRoutes::new([
            ("specify.build.>".to_owned(), id("workflow")),
            ("events.*.created".to_owned(), id("audit")),
        ]);
        assert_eq!(routes.resolve("specify.build.rust"), Some(&id("workflow")));
        assert_eq!(routes.resolve("specify.build.rust.extra"), Some(&id("workflow")));
        assert_eq!(routes.resolve("events.user.created"), Some(&id("audit")));
        assert_eq!(routes.resolve("events.user.deleted"), None);
        assert_eq!(routes.resolve("specify.build"), None);
    }

    #[test]
    fn build_catch_all() {
        let router = Router::build("http", &[id("only")], HttpRoutes::default())
            .expect("a sole exporter is the catch-all");
        assert_eq!(router.resolve("/anything"), Some(&id("only")));
    }

    #[test]
    fn build_inert() {
        let router = Router::build("http", &[], HttpRoutes::default())
            .expect("no exporters is inert, not an error");
        assert!(router.is_inert());
        assert_eq!(router.resolve("/anything"), None);
    }

    #[test]
    fn build_ambiguous() {
        let error = Router::build("http", &[id("a"), id("b")], HttpRoutes::default())
            .expect_err("two exporters with no routes is ambiguous");
        assert!(error.to_string().contains("2 capable guests"));
    }

    #[test]
    fn build_routes() {
        let routes = HttpRoutes::new([("/a".to_owned(), id("a"))]);
        let r = Router::build("http", &[id("a")], routes).expect("routes are valid");
        assert_eq!(r.resolve("/a"), Some(&id("a")));
        // An explicit route makes the trigger fully route-driven: a miss is a
        // miss even though `a` is the sole exporter.
        assert_eq!(r.resolve("/b"), None);
    }

    #[test]
    fn reject_route() {
        let routes = HttpRoutes::new([("/a".to_owned(), id("ghost"))]);
        let error = Router::build("http", &[id("real")], routes)
            .expect_err("a route to a non-exporter must fail fast");
        assert!(error.to_string().contains("ghost"));
    }

    #[test]
    fn trigger_router_catch_all() {
        let router = Router::build("http", &[id("a")], HttpRoutes::default())
            .expect("a sole exporter is the catch-all");
        let tr = TriggerRouter {
            indices: HashMap::from([(id("a"), 7u32)]),
            router,
        };
        assert!(!tr.is_inert());
        assert_eq!(tr.resolve("/anything"), Some((&id("a"), &7u32)));
        assert_eq!(tr.catch_all(), Some((&id("a"), &7u32)));
    }

    #[test]
    fn trigger_router_routed() {
        let routes = HttpRoutes::new([("/a".to_owned(), id("a"))]);
        let r = Router::build("http", &[id("a")], routes).expect("routes are valid");
        let tr = TriggerRouter {
            indices: HashMap::from([(id("a"), 1u32)]),
            router: r,
        };
        assert_eq!(tr.resolve("/a"), Some((&id("a"), &1u32)));
        assert_eq!(tr.resolve("/miss"), None);
        // An explicit route table has no unkeyed catch-all.
        assert_eq!(tr.catch_all(), None);
    }

    #[test]
    fn trigger_router_inert() {
        let router =
            Router::build("http", &[], HttpRoutes::default()).expect("no exporters is inert");
        let tr: TriggerRouter<u32, HttpRoutes> = TriggerRouter {
            indices: HashMap::new(),
            router,
        };
        assert!(tr.is_inert());
        assert_eq!(tr.resolve("/anything"), None);
        assert_eq!(tr.catch_all(), None);
    }

    #[test]
    fn trigger_router_missing_index() {
        // Defensive: the router resolves an identity whose indices entry is
        // absent (never happens post-build, since both come from the same set).
        let router = Router::build("http", &[id("a")], HttpRoutes::default())
            .expect("a sole exporter is the catch-all");
        let tr: TriggerRouter<u32, HttpRoutes> = TriggerRouter {
            indices: HashMap::new(),
            router,
        };
        assert_eq!(tr.resolve("/anything"), None);
        assert_eq!(tr.catch_all(), None);
    }
}
