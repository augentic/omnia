//! Per-guest grants: what of the deployment one guest reaches.

use serde::Deserialize;

use crate::{egress, names};

/// What of the deployment one guest reaches: host variables, mounts, host
/// names, other guests, and the packages it imports.
///
/// Each list is `None` to reach everything of its kind, or the entries it
/// reaches. A [`hosts`](Self::hosts) entry is a whole host name, or
/// `*.suffix`, as [`host_pattern`](crate::host_pattern) checks. Any other
/// entry is a whole name, or a prefix that ends in `*`, so `acme:*` covers
/// every guest or package of the `acme` namespace.
///
/// - [`Grants::all`], the default, reaches everything the deployment has. A
///   guest that declares no grants runs under it.
/// - A declared table starts from [`Grants::none`]. A list it omits reaches
///   nothing, except [`interfaces`](Self::interfaces), which reaches every
///   package until the table names some.
///
/// A builder method adds its entries to a list, and starts a list that
/// reached everything with them.
///
/// # Examples
///
/// ```
/// use omnia_core::Grants;
///
/// let grants = Grants::none().env(["ORDERS_*"]).mounts(["data"]);
/// assert!(grants.allows_env("ORDERS_URL"));
/// assert!(!grants.allows_env("HOME"));
/// assert!(!grants.allows_guest("pricing"));
/// assert!(grants.allows_interface("wasi:http"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "Table")]
pub struct Grants {
    /// The host variables the guest sees, of those `[environment] inherit`
    /// passes.
    ///
    /// The runtime's own `RUST_LOG` and `NO_COLOR` reach the guest whatever
    /// this lists.
    pub env: Option<Vec<String>>,
    /// The `[[mount]]`s preopened into the guest, by name.
    pub mounts: Option<Vec<String>>,
    /// The host names the guest states, wherever it states one.
    ///
    /// - The host of each outbound `wasi:http` request, and of each redirect
    ///   hop the runtime follows for it.
    /// - Each `egress` entry of a model worker grant.
    /// - The registry a plugin load names.
    ///
    /// Under a list, a guest reaches a host by name alone, on any port,
    /// since an IP literal matches no entry.
    pub hosts: Option<Vec<String>>,
    /// The guests the guest calls over a link or loads by name.
    pub guests: Option<Vec<String>>,
    /// The packages the guest may import, such as `wasi:http` or
    /// `acme:pricing`.
    ///
    /// An import with no function and no resource needs no entry.
    pub interfaces: Option<Vec<String>>,
}

impl Grants {
    /// Grants that reach everything the deployment has.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Grants that reach no host variable, mount, host name, or guest, and
    /// every package.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            env: Some(Vec::new()),
            mounts: Some(Vec::new()),
            hosts: Some(Vec::new()),
            guests: Some(Vec::new()),
            interfaces: None,
        }
    }

    /// Lists `names` among the host variables these grants reach.
    #[must_use]
    pub fn env<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        add(&mut self.env, names);
        self
    }

    /// Lists `names` among the mounts these grants reach.
    #[must_use]
    pub fn mounts<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        add(&mut self.mounts, names);
        self
    }

    /// Lists `patterns` among the host names these grants reach.
    #[must_use]
    pub fn hosts<S: Into<String>>(mut self, patterns: impl IntoIterator<Item = S>) -> Self {
        add(&mut self.hosts, patterns);
        self
    }

    /// Lists `names` among the guests these grants reach.
    #[must_use]
    pub fn guests<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        add(&mut self.guests, names);
        self
    }

    /// Lists `packages` among the packages these grants reach.
    #[must_use]
    pub fn interfaces<S: Into<String>>(mut self, packages: impl IntoIterator<Item = S>) -> Self {
        add(&mut self.interfaces, packages);
        self
    }

    /// Whether these grants reach the host variable `name`.
    #[must_use]
    pub fn allows_env(&self, name: &str) -> bool {
        allows(self.env.as_deref(), name)
    }

    /// Whether these grants reach the mount `name`.
    #[must_use]
    pub fn allows_mount(&self, name: &str) -> bool {
        allows(self.mounts.as_deref(), name)
    }

    /// Whether these grants reach the host name `host`, such as a request
    /// URL's host.
    #[must_use]
    pub fn allows_host(&self, host: &str) -> bool {
        self.hosts
            .as_deref()
            .is_none_or(|hosts| hosts.iter().any(|pattern| egress::matches_host(pattern, host)))
    }

    /// Whether these grants reach every host the host pattern `entry`
    /// matches, such as a model worker's `egress` entry.
    #[must_use]
    pub fn covers_host(&self, entry: &str) -> bool {
        self.hosts
            .as_deref()
            .is_none_or(|hosts| hosts.iter().any(|pattern| egress::covers_host(pattern, entry)))
    }

    /// Whether these grants reach the guest `name`.
    #[must_use]
    pub fn allows_guest(&self, name: &str) -> bool {
        allows(self.guests.as_deref(), name)
    }

    /// Whether these grants reach the package `package`, such as `wasi:http`.
    #[must_use]
    pub fn allows_interface(&self, package: &str) -> bool {
        allows(self.interfaces.as_deref(), package)
    }

    /// Whether these grants reach everything, as [`Grants::all`] does.
    #[must_use]
    pub const fn is_all(&self) -> bool {
        self.env.is_none()
            && self.mounts.is_none()
            && self.hosts.is_none()
            && self.guests.is_none()
            && self.interfaces.is_none()
    }

    /// The grants that reach only what both these and `outer` reach.
    #[must_use]
    pub fn within(&self, outer: &Self) -> Self {
        let meet_names = |inner: &Option<Vec<String>>, outer: &Option<Vec<String>>| {
            meet(inner.as_deref(), outer.as_deref(), names::narrower)
        };
        Self {
            env: meet_names(&self.env, &outer.env),
            mounts: meet_names(&self.mounts, &outer.mounts),
            hosts: meet(self.hosts.as_deref(), outer.hosts.as_deref(), egress::narrower_host),
            guests: meet_names(&self.guests, &outer.guests),
            interfaces: meet_names(&self.interfaces, &outer.interfaces),
        }
    }

    /// The first [`mounts`](Self::mounts) entry that matches none of the
    /// `declared` mount names.
    #[must_use]
    pub fn undeclared_mount<'a>(&'a self, declared: &[&str]) -> Option<&'a str> {
        self.mounts
            .iter()
            .flatten()
            .map(String::as_str)
            .find(|entry| !declared.iter().any(|name| names::matches(entry, name)))
    }
}

fn add<S: Into<String>>(list: &mut Option<Vec<String>>, names: impl IntoIterator<Item = S>) {
    list.get_or_insert_default().extend(names.into_iter().map(Into::into));
}

fn allows(list: Option<&[String]>, name: &str) -> bool {
    list.is_none_or(|list| list.iter().any(|pattern| names::matches(pattern, name)))
}

// two patterns meet in the narrower of them, or not at all, so the meeting
// of two lists is exact
fn meet(
    inner: Option<&[String]>, outer: Option<&[String]>,
    narrower: for<'a> fn(&'a str, &'a str) -> Option<&'a str>,
) -> Option<Vec<String>> {
    match (inner, outer) {
        (None, None) => None,
        (Some(list), None) | (None, Some(list)) => Some(list.to_vec()),
        (Some(inner), Some(outer)) => Some(
            inner
                .iter()
                .flat_map(|a| outer.iter().filter_map(move |b| narrower(a, b)))
                .map(str::to_owned)
                .collect(),
        ),
    }
}

// A declared table: an omitted list reaches nothing, and omitted
// `interfaces` reach every package.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    mounts: Vec<String>,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    guests: Vec<String>,
    interfaces: Option<Vec<String>>,
}

impl From<Table> for Grants {
    fn from(table: Table) -> Self {
        Self {
            env: Some(table.env),
            mounts: Some(table.mounts),
            hosts: Some(table.hosts),
            guests: Some(table.guests),
            interfaces: table.interfaces,
        }
    }
}

// Unit tests by design: meeting two lists and matching host patterns are
// pure, and no single guest boundary reaches every shape of pattern pair.
#[cfg(test)]
mod tests {
    use super::Grants;

    #[test]
    fn within_all() {
        let declared = Grants::none().env(["ORDERS_*"]).guests(["acme:*"]);
        assert_eq!(declared.within(&Grants::all()), declared);
        assert_eq!(Grants::all().within(&declared), declared);
        assert!(Grants::all().within(&Grants::all()).is_all());
    }

    #[test]
    fn within_lists() {
        let inner = Grants::none()
            .env(["ORDERS_*", "HOME", "PATH"])
            .hosts(["*.example.com", "api.example.net"])
            .guests(["acme:*", "pricing"])
            .interfaces(["wasi:*"]);
        let outer = Grants::none()
            .env(["ORDERS_URL", "ORD*", "HOME*"])
            .hosts(["api.example.com", "*.cdn.example.com", "*.example.net"])
            .guests(["acme:tool", "billing"])
            .interfaces(["wasi:http", "omnia:otel"]);
        let met = inner.within(&outer);

        assert_eq!(met.env, Some(list(&["ORDERS_URL", "ORDERS_*", "HOME"])));
        assert_eq!(
            met.hosts,
            Some(list(&["api.example.com", "*.cdn.example.com", "api.example.net"]))
        );
        assert_eq!(met.guests, Some(list(&["acme:tool"])));
        assert_eq!(met.interfaces, Some(list(&["wasi:http"])));
        assert_eq!(met.mounts, Some(list(&[])));
    }

    #[test]
    fn host_patterns() {
        let grants = Grants::none().hosts(["*.example.com", "api.example.net"]);
        for host in ["example.com", "a.example.com", "A.B.Example.COM", "api.example.net"] {
            assert!(grants.allows_host(host), "`{host}` is reached");
        }
        for host in ["badexample.com", "example.net", "x.api.example.net", "127.0.0.1", "[::1]"] {
            assert!(!grants.allows_host(host), "`{host}` is not reached");
        }
        for entry in ["example.com", "*.a.example.com", "api.example.net"] {
            assert!(grants.covers_host(entry), "`{entry}` is covered");
        }
        for entry in ["*.example.net", "*.api.example.net", "*.com"] {
            assert!(!grants.covers_host(entry), "`{entry}` is not covered");
        }
        assert!(Grants::all().allows_host("127.0.0.1"), "no list reaches every host");
    }

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }
}
