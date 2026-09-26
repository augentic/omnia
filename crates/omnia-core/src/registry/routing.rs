//! # Trigger routing
//!
//! Maps an inbound trigger to a guest [`GuestId`] generically: the runtime core routes
//! on opaque identities and route strings only, never on domain concepts.
//!
//! [`RouteTable`] is one table generic over a [`MatchStrategy`]; the aliases
//! [`HttpRoutes`] (longest-prefix), [`PatternRoutes`] (NATS-style tokens, also
//! driving websocket routes), and [`CliRoutes`] (exact subcommand) select the
//! policy.
//!
//! [`Router`] layers the capability-based default routing of the guest-registry
//! design over a table: with no routes configured a sole handler exporter among
//! the guests loaded at boot is the catch-all for its trigger, zero exporters is
//! inert, and two or more exporters require explicit routes to disambiguate. A
//! route may also name a guest the deployment declares but loads at first use;
//! whether it exports the handler is learnt when it loads.

use std::marker::PhantomData;

use anyhow::{Result, bail};
use wasmtime::component::InstancePre;

use super::{GuestId, Registry};

/// A per-trigger route table resolving a routing key to a target identity.
pub trait Resolver {
    /// Resolve a routing key (a path, topic, ...) to a target guest identity.
    fn resolve(&self, key: &str) -> Option<&GuestId>;

    /// Iterate the identities every route in the table targets.
    fn targets(&self) -> impl Iterator<Item = &GuestId>;

    /// Returns `true` when the table holds no routes.
    fn is_empty(&self) -> bool;
}

/// Match-and-order policy that distinguishes the route tables.
pub trait MatchStrategy {
    /// Whether `key` matches route `pattern`.
    fn matches(key: &str, pattern: &str) -> bool;

    /// Reorder entries at construction so a linear `resolve` scan returns the
    /// intended winner. Defaults to preserving declaration order.
    fn order(_entries: &mut [(String, GuestId)]) {}
}

/// Longest-prefix path matching: `/target/omnia` wins over `/target`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrefixMatch;

/// NATS-style wildcard matching: `.`-tokenised, `*` matches one token, `>`
/// matches one or more trailing tokens.
#[derive(Clone, Copy, Debug, Default)]
pub struct PatternMatch;

/// Exact string matching, used for CLI subcommands.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExactMatch;

impl MatchStrategy for PrefixMatch {
    fn matches(key: &str, pattern: &str) -> bool {
        path_has_prefix(key, pattern)
    }

    fn order(entries: &mut [(String, GuestId)]) {
        entries.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
    }
}

impl MatchStrategy for PatternMatch {
    fn matches(key: &str, pattern: &str) -> bool {
        topic_matches(key, pattern)
    }
}

impl MatchStrategy for ExactMatch {
    fn matches(key: &str, pattern: &str) -> bool {
        pattern == key
    }
}

/// A per-trigger route table resolving a routing key to a target identity,
/// generic over its [`MatchStrategy`].
#[derive(Clone, Debug, Default)]
pub struct RouteTable<M> {
    // ordered by the strategy at construction
    entries: Vec<(String, GuestId)>,
    strategy: PhantomData<M>,
}

impl<M: MatchStrategy> RouteTable<M> {
    /// Build a table from `(pattern, target)` pairs, applying the strategy's
    /// construction ordering so `resolve` is a single linear scan.
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = (String, GuestId)>) -> Self {
        let mut entries: Vec<(String, GuestId)> = entries.into_iter().collect();
        M::order(&mut entries);
        Self {
            entries,
            strategy: PhantomData,
        }
    }
}

impl<M: MatchStrategy> Resolver for RouteTable<M> {
    fn resolve(&self, key: &str) -> Option<&GuestId> {
        self.entries.iter().find(|(pattern, _)| M::matches(key, pattern)).map(|(_, id)| id)
    }

    fn targets(&self) -> impl Iterator<Item = &GuestId> {
        self.entries.iter().map(|(_, id)| id)
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Longest-prefix HTTP route table.
pub type HttpRoutes = RouteTable<PrefixMatch>;

/// NATS-style wildcard route table, driving messaging topics and websocket
/// routes.
pub type PatternRoutes = RouteTable<PatternMatch>;

/// Exact-match CLI subcommand route table.
///
/// CLI route parsing is not yet wired, so this table is empty today and a sole
/// `wasi:cli/run` exporter is the catch-all; the type exists so the `cli`
/// trigger routes through the same machinery as every other trigger.
pub type CliRoutes = RouteTable<ExactMatch>;

/// The per-trigger route tables a registry carries, aggregated from the
/// manifest guests' `routes` lists.
#[derive(Clone, Debug, Default)]
pub struct Routes {
    http: HttpRoutes,
    messaging: PatternRoutes,
    websocket: PatternRoutes,
    cli: CliRoutes,
}

impl Routes {
    /// Assemble the per-trigger tables.
    #[must_use]
    pub const fn new(
        http: HttpRoutes, messaging: PatternRoutes, websocket: PatternRoutes, cli: CliRoutes,
    ) -> Self {
        Self {
            http,
            messaging,
            websocket,
            cli,
        }
    }

    /// The HTTP (longest-prefix) route table.
    #[must_use]
    pub const fn http(&self) -> &HttpRoutes {
        &self.http
    }

    /// The messaging (topic) route table.
    #[must_use]
    pub const fn messaging(&self) -> &PatternRoutes {
        &self.messaging
    }

    /// The websocket (route) route table.
    #[must_use]
    pub const fn websocket(&self) -> &PatternRoutes {
        &self.websocket
    }

    /// The CLI (subcommand) route table.
    #[must_use]
    pub const fn cli(&self) -> &CliRoutes {
        &self.cli
    }

    /// Iterate every identity any route targets across all triggers — used to
    /// validate routes name registered guests.
    pub fn targets(&self) -> impl Iterator<Item = &GuestId> {
        self.http
            .targets()
            .chain(self.messaging.targets())
            .chain(self.websocket.targets())
            .chain(self.cli.targets())
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
    /// Decide how `trigger` routes, given the loaded guests that export its
    /// handler (`capable`, in a stable order), which identities the
    /// deployment `declares` for loading at first use, and the configured
    /// `table`.
    ///
    /// With routes configured the trigger is fully route-driven, and every
    /// target must be capable or declared: a declared target's handler is
    /// checked when it loads. With an empty table the capability default
    /// routes by exporter count: one is the catch-all, none is inert, two or
    /// more is ambiguous. A declared guest is never the catch-all.
    ///
    /// # Errors
    ///
    /// Returns an error if a route targets a loaded guest that does not
    /// export the handler or an identity the deployment does not declare, or
    /// if two or more guests export it with no routes.
    pub fn build(
        trigger: &str, capable: &[GuestId], declares: impl Fn(&GuestId) -> bool, resolver: R,
    ) -> Result<Self> {
        if !resolver.is_empty() {
            return Self::routed(trigger, capable, declares, resolver);
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

    fn routed(
        trigger: &str, capable: &[GuestId], declares: impl Fn(&GuestId) -> bool, resolver: R,
    ) -> Result<Self> {
        for target in resolver.targets() {
            if !capable.contains(target) && !declares(target) {
                bail!(
                    "route for trigger `{trigger}` names `{target}`, which does not export the \
                     `{trigger}` handler"
                );
            }
        }
        Ok(Self::Routed(resolver))
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

/// A per-trigger router built over a registry.
///
/// A trigger server builds this once at boot, then resolves each routing key
/// to the identity it fetches through the runtime's first-use seam — a
/// declared guest loads the first time a key names it — and probes for the
/// typed binding indices it needs to instantiate.
pub struct TriggerRouter<R> {
    router: Router<R>,
}

impl<R: Resolver> TriggerRouter<R> {
    /// Probe every registered guest for the trigger's handler — a guest is
    /// *capable* exactly when `probe` succeeds — then build the `Router` over
    /// the capable set, the registry's declared identities, and the
    /// configured route `table`.
    ///
    /// # Errors
    ///
    /// Returns an error if `Router::build` rejects the capable set and
    /// table: a route names a guest that neither exports the handler nor is
    /// declared, or two or more guests export it with no routes.
    pub fn build<T, I, E, F>(
        registry: &Registry<T>, trigger: &str, table: R, mut probe: F,
    ) -> Result<Self>
    where
        F: FnMut(&InstancePre<T>) -> Result<I, E>,
    {
        let capable: Vec<GuestId> = registry
            .guests()
            .filter(|guest| probe(guest.instance_pre()).is_ok())
            .map(|guest| guest.id().clone())
            .collect();
        let router = Router::build(trigger, &capable, |id| registry.is_declared(id), table)?;
        Ok(Self { router })
    }

    /// Returns `true` when no guest answers this trigger.
    #[must_use]
    pub const fn is_inert(&self) -> bool {
        self.router.is_inert()
    }

    /// Resolve a routing `key` to the target identity, or `None` on a miss or
    /// an inert trigger. A catch-all ignores the key.
    #[must_use]
    pub fn resolve(&self, key: &str) -> Option<&GuestId> {
        self.router.resolve(key)
    }

    /// The sole-exporter catch-all target, if this trigger fans an unkeyed
    /// call into a single exporter (used by websocket events that carry no
    /// route).
    #[must_use]
    pub const fn catch_all(&self) -> Option<&GuestId> {
        self.router.catch_all()
    }
}

// segment-aware: `/a` matches `/a` and `/a/b` but not `/ab`
fn path_has_prefix(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || prefix.ends_with('/'))
}

// NATS-style: `*` matches exactly one `.`-token, a trailing `>` one or more
fn topic_matches(topic: &str, pattern: &str) -> bool {
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
    fn http_longest_prefix() {
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
        let routes = PatternRoutes::new([
            ("specify.build.>".to_owned(), id("workflow")),
            ("events.*.created".to_owned(), id("audit")),
        ]);
        assert_eq!(routes.resolve("specify.build.rust"), Some(&id("workflow")));
        assert_eq!(routes.resolve("specify.build.rust.extra"), Some(&id("workflow")));
        assert_eq!(routes.resolve("events.user.created"), Some(&id("audit")));
        assert_eq!(routes.resolve("events.user.deleted"), None);
        assert_eq!(routes.resolve("specify.build"), None);
    }

    fn none_declared(_: &GuestId) -> bool {
        false
    }

    #[test]
    fn build_catch_all() {
        let router = Router::build("http", &[id("only")], none_declared, HttpRoutes::default())
            .expect("a sole exporter is the catch-all");
        assert_eq!(router.resolve("/anything"), Some(&id("only")));
    }

    #[test]
    fn build_inert() {
        let router = Router::build("http", &[], none_declared, HttpRoutes::default())
            .expect("no exporters is inert, not an error");
        assert!(router.is_inert());
        assert_eq!(router.resolve("/anything"), None);
    }

    #[test]
    fn declared_is_never_catch_all() {
        let router =
            Router::build("http", &[], |guest| *guest == id("later"), HttpRoutes::default())
                .expect("no loaded exporter is inert, whatever is declared");
        assert!(router.is_inert());
    }

    #[test]
    fn build_ambiguous() {
        let error =
            Router::build("http", &[id("a"), id("b")], none_declared, HttpRoutes::default())
                .expect_err("two exporters with no routes is ambiguous");
        assert!(error.to_string().contains("2 capable guests"));
    }

    #[test]
    fn build_routes() {
        let routes = HttpRoutes::new([("/a".to_owned(), id("a"))]);
        let r = Router::build("http", &[id("a")], none_declared, routes).expect("routes are valid");
        assert_eq!(r.resolve("/a"), Some(&id("a")));
        // an explicit route makes the trigger route-driven: a miss is a miss
        assert_eq!(r.resolve("/b"), None);
    }

    #[test]
    fn defer_declared_route() {
        let routes = HttpRoutes::new([("/a".to_owned(), id("later"))]);
        let r = Router::build("http", &[], |guest| *guest == id("later"), routes)
            .expect("a route may name a declared guest that has not loaded");
        assert_eq!(r.resolve("/a"), Some(&id("later")));
    }

    #[test]
    fn reject_route() {
        let routes = HttpRoutes::new([("/a".to_owned(), id("ghost"))]);
        let error = Router::build("http", &[id("real")], none_declared, routes)
            .expect_err("a route to a non-exporter must fail fast");
        assert!(error.to_string().contains("ghost"));
    }
}
