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

use anyhow::{Result, bail};

use crate::registry::GuestId;

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

/// Path-prefix match that respects segment boundaries: `/a` matches `/a` and
/// `/a/b` but not `/ab`.
fn path_has_prefix(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || prefix.ends_with('/'))
}

/// NATS-style topic match: `.`-tokenised, `*` matches exactly one token, and `>`
/// matches one or more trailing tokens (and must be the final pattern token).
fn topic_matches(topic: &str, pattern: &str) -> bool {
    let topic_tokens: Vec<&str> = topic.split('.').collect();
    let pattern_tokens: Vec<&str> = pattern.split('.').collect();

    for (i, token) in pattern_tokens.iter().enumerate() {
        match *token {
            ">" => return i == pattern_tokens.len() - 1 && i < topic_tokens.len(),
            "*" => {
                if i >= topic_tokens.len() {
                    return false;
                }
            }
            literal => {
                if topic_tokens.get(i) != Some(&literal) {
                    return false;
                }
            }
        }
    }

    pattern_tokens.len() == topic_tokens.len()
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
}
