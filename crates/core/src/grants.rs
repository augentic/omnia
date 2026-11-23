//! Per-guest grants: what of the deployment one guest reaches.

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

use crate::names;

/// What of the deployment one guest reaches: host variables, mounts, and the packages it imports.
///
/// Each list either reaches everything of its kind or holds the entries it
/// reaches. An entry is a whole name, or a prefix that ends in `*`, so
/// `wasi:*` covers every package of the `wasi` namespace.
///
/// - [`Grants::all`] reaches everything the deployment has. A guest that
///   declares no grants runs under it.
/// - A declared table starts from [`Grants::none`], so a list it omits
///   reaches nothing.
///
/// A builder method adds its entries to a list, and starts a list that
/// reached everything with them. A list is kept sorted and deduplicated, so
/// grants that list the same entries in another order are equal.
///
/// # Examples
///
/// ```
/// use omnia_core::Grants;
///
/// let grants = Grants::none().env(["ORDERS_*"]).mounts(["data"]);
/// assert!(grants.allows_env("ORDERS_URL"));
/// assert!(!grants.allows_env("HOME"));
/// assert!(!grants.allows_interface("wasi:http"));
///
/// let grants = grants.interfaces(["wasi:*"]);
/// assert!(grants.allows_interface("wasi:http"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "Table")]
pub struct Grants {
    env: Option<Vec<String>>,
    mounts: Option<Vec<String>>,
    interfaces: Option<Vec<String>>,
}

impl Grants {
    /// Grants that reach everything the deployment has.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            env: None,
            mounts: None,
            interfaces: None,
        }
    }

    /// Grants that reach no host variable, no mount, and no package.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            env: Some(Vec::new()),
            mounts: Some(Vec::new()),
            interfaces: Some(Vec::new()),
        }
    }

    /// Lists `names` among the host variables these grants reach.
    ///
    /// The guest sees a host variable only when the deployment inherits it
    /// and this list names it. The runtime's own `RUST_LOG` and `NO_COLOR`
    /// reach the guest whatever the list says. The list holds the guest's
    /// `wasi:cli` environment alone: its `omnia:config` reads are held by
    /// [`interfaces`](Self::interfaces), and served by the config backend.
    #[must_use]
    pub fn env<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        add(&mut self.env, names);
        self
    }

    /// Lists `names` among the mounts these grants reach.
    ///
    /// The guest's sandbox preopens only the deployment's mounts the list
    /// names.
    #[must_use]
    pub fn mounts<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        add(&mut self.mounts, names);
        self
    }

    /// Lists `packages` among the packages these grants reach.
    ///
    /// An entry is `*`, a package such as `wasi:http` or `acme:pricing`, or
    /// a prefix of one that ends in `*`. Admission refuses a component that
    /// imports a function or a resource from a package the list does not
    /// reach. An import of types alone needs no entry.
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

    /// Whether these grants reach the package `package`, such as `wasi:http`.
    #[must_use]
    pub fn allows_interface(&self, package: &str) -> bool {
        allows(self.interfaces.as_deref(), package)
    }

    /// Whether these grants reach everything, as [`Grants::all`] does.
    #[must_use]
    pub const fn is_all(&self) -> bool {
        self.env.is_none() && self.mounts.is_none() && self.interfaces.is_none()
    }

    /// The grants that reach exactly what both these and `outer` reach.
    ///
    /// A list keeps no entry that another of its entries covers, so two
    /// narrowings that reach the same names are equal. Two lists with
    /// nothing in common narrow to an empty list, which reaches nothing.
    #[must_use]
    pub fn within(&self, outer: &Self) -> Self {
        Self {
            env: meet(self.env.as_deref(), outer.env.as_deref()),
            mounts: meet(self.mounts.as_deref(), outer.mounts.as_deref()),
            interfaces: meet(self.interfaces.as_deref(), outer.interfaces.as_deref()),
        }
    }

    /// Checks every entry against the deployment the guest runs in.
    ///
    /// `mounts` are the names of the deployment's mounts, and `inherit` the
    /// host variables it inherits, each a whole name or a prefix that ends
    /// in `*`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first entry that:
    ///
    /// - in [`env`](Self::env), is not a variable [`pattern`](crate::env::pattern),
    ///   or reaches no variable `inherit` names;
    /// - in [`mounts`](Self::mounts), names none of `mounts`;
    /// - in [`interfaces`](Self::interfaces), is not `*`, a package such as
    ///   `wasi:http`, or a prefix of one that ends in `*`.
    pub fn validate(&self, mounts: &[&str], inherit: &[String]) -> Result<()> {
        for entry in self.env.iter().flatten() {
            crate::env::pattern(entry).map_err(|error| anyhow!("`env` entry {error}"))?;
            if !inherit.iter().any(|inherited| names::narrower(entry, inherited).is_some()) {
                bail!("`env` entry `{entry}` reaches no variable the deployment inherits");
            }
        }
        for entry in self.mounts.iter().flatten() {
            if !mounts.iter().any(|mount| names::matches(entry, mount)) {
                bail!("`mounts` entry `{entry}` names no mount of the deployment");
            }
        }
        for entry in self.interfaces.iter().flatten() {
            if is_label(entry) {
                bail!(
                    "`interfaces` entry `{entry}` is a namespace, not a package: write `{entry}:*` \
                     for every package in it"
                );
            }
            if !is_package_pattern(entry) {
                bail!(
                    "`interfaces` entry `{entry}` is not a package name (`wasi:http`), or a \
                     prefix of one that ends in `*`"
                );
            }
        }
        Ok(())
    }
}

fn add<S: Into<String>>(list: &mut Option<Vec<String>>, names: impl IntoIterator<Item = S>) {
    let list = list.get_or_insert_default();
    list.extend(names.into_iter().map(Into::into));
    list.sort_unstable();
    list.dedup();
}

fn allows(list: Option<&[String]>, name: &str) -> bool {
    list.is_none_or(|list| list.iter().any(|pattern| names::matches(pattern, name)))
}

// two patterns meet in the narrower of them, or not at all, so the meeting of
// two lists is exact
fn meet(inner: Option<&[String]>, outer: Option<&[String]>) -> Option<Vec<String>> {
    let met: Vec<&str> = match (inner, outer) {
        (None, None) => return None,
        (Some(list), None) | (None, Some(list)) => list.iter().map(String::as_str).collect(),
        (Some(inner), Some(outer)) => inner
            .iter()
            .flat_map(|a| outer.iter().filter_map(move |b| names::narrower(a, b)))
            .collect(),
    };
    let mut list = None;
    add(
        &mut list,
        met.iter()
            .copied()
            .filter(|entry| !met.iter().any(|other| other != entry && names::covers(other, entry))),
    );
    list
}

// `*`, a package of two WIT labels (`wasi:http`), or a prefix of one that
// ends in `*`
fn is_package_pattern(entry: &str) -> bool {
    let Some(prefix) = entry.strip_suffix('*') else {
        return entry
            .split_once(':')
            .is_some_and(|(namespace, package)| is_label(namespace) && is_label(package));
    };
    match prefix.split_once(':') {
        Some((namespace, package)) => is_label(namespace) && is_label_prefix(package),
        None => is_label_prefix(prefix),
    }
}

// every prefix of a label is empty, a label, or a label and the `-` before
// its next word
fn is_label_prefix(text: &str) -> bool {
    text.is_empty() || is_label(text) || text.strip_suffix('-').is_some_and(is_label)
}

// WIT's `word ('-' word)*`, each word lowercase or uppercase throughout
fn is_label(text: &str) -> bool {
    text.split('-').all(|word| {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) if first.is_ascii_lowercase() => {
                chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            }
            Some(first) if first.is_ascii_uppercase() => {
                chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            }
            _ => false,
        }
    })
}

// A declared table: a list it omits reaches nothing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    mounts: Vec<String>,
    #[serde(default)]
    interfaces: Vec<String>,
}

impl From<Table> for Grants {
    fn from(table: Table) -> Self {
        Self::none().env(table.env).mounts(table.mounts).interfaces(table.interfaces)
    }
}

// Unit tests by design: validation and narrowing are pure functions of the
// grants and the deployment's names, and no one deployment reaches every
// shape they handle.
#[cfg(test)]
mod tests {
    use super::Grants;

    #[test]
    fn shapes() {
        let mounts = ["data", "logs"];
        let inherit = ["ORDERS_*".to_owned(), "HOME".to_owned()];
        Grants::none()
            .env(["ORDERS_URL", "ORD*", "HOME", "*"])
            .mounts(["data", "lo*"])
            .interfaces(["*", "wasi:*", "wasi:http", "omnia-test:*", "acme:pri*", "wasi*"])
            .validate(&mounts, &inherit)
            .expect("every entry is valid");
        Grants::all().validate(&[], &[]).expect("no list holds an entry");

        let refused = [
            (Grants::none().mounts(["dta"]), "`mounts` entry `dta`"),
            (Grants::none().env(["ORDERS-URL"]), "`env` entry `ORDERS-URL`"),
            (Grants::none().env(["PATH"]), "`env` entry `PATH` reaches no variable"),
            (Grants::none().interfaces(["wasi"]), "write `wasi:*`"),
        ];
        for (grants, needle) in refused {
            let error = grants.validate(&mounts, &inherit).expect_err("an entry is refused");
            assert!(error.to_string().contains(needle), "{error}");
        }

        for entry in ["", "**", "a*b", "Wasi:Cli", "wasi:cli/environment", "wasi:cli@0.3.0"] {
            let error = Grants::none()
                .interfaces([entry])
                .validate(&mounts, &inherit)
                .expect_err("not a package pattern");
            assert!(
                error.to_string().starts_with(&format!("`interfaces` entry `{entry}` is not")),
                "{error}"
            );
        }
    }

    #[test]
    fn within_all() {
        let declared = Grants::none().env(["ORDERS_*"]).interfaces(["wasi:*"]);
        assert_eq!(declared.within(&Grants::all()), declared);
        assert_eq!(Grants::all().within(&declared), declared);
        assert!(Grants::all().within(&Grants::all()).is_all());
    }

    #[test]
    fn within_lists() {
        let inner = Grants::none()
            .env(["ORDERS_*", "HOME", "PATH"])
            .mounts(["data"])
            .interfaces(["wasi:*"]);
        let outer = Grants::none()
            .env(["ORDERS_URL", "ORD*", "HOME*"])
            .mounts(["logs"])
            .interfaces(["wasi:http", "omnia:otel"]);
        let met = inner.within(&outer);

        // `ORDERS_URL` also meets `ORDERS_*`, which covers it, and the
        // disjoint `mounts` meet in the empty list `Grants::none()` holds
        assert_eq!(met, Grants::none().env(["ORDERS_*", "HOME"]).interfaces(["wasi:http"]));
        assert_eq!(outer.within(&inner), met);
    }
}
