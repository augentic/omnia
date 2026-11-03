//! Service contract for plugin package stores.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use futures::future::BoxFuture;
use wasm_pkg_common::package::{PackageRef, Version};

/// An exact package release: a `namespace:name@version` reference.
///
/// Filed in a store as `namespace_name@version.wasm` — the one spelling
/// `wkg get <reference> -o <dir>/` writes, which a wasm-pkg label (`[a-z0-9]`
/// words joined by `-`) and a semver leave invertible: a label holds neither
/// `_` nor `@`, and a version holds no `_`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    package: PackageRef,
    version: Version,
}

impl Reference {
    /// The package without its version.
    #[must_use]
    pub const fn package(&self) -> &PackageRef {
        &self.package
    }

    /// The exact version.
    #[must_use]
    pub const fn version(&self) -> &Version {
        &self.version
    }

    /// The file a store files this release as: `namespace_name@version.wasm`.
    #[must_use]
    pub fn file_name(&self) -> String {
        format!("{}_{}@{}.wasm", self.package.namespace(), self.package.name(), self.version)
    }
}

impl FromStr for Reference {
    type Err = anyhow::Error;

    fn from_str(reference: &str) -> Result<Self> {
        let Some((name, version)) = reference.split_once('@') else {
            bail!(
                "registry package `{reference}` must pin an exact version \
                 (`namespace:name@version`)"
            )
        };
        let package = name.parse().with_context(|| {
            format!("package `{reference}` is not a `namespace:name@version` reference")
        })?;
        let version = version.parse().with_context(|| {
            format!("package `{reference}` does not pin an exact semver version")
        })?;
        Ok(Self { package, version })
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.package, self.version)
    }
}

/// Where acquired packages are kept, keyed by exact [`Reference`].
pub trait PackageStore: Send + Sync + 'static {
    /// The stored bytes of `reference`, if the store holds it.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read.
    fn get<'a>(&'a self, reference: &'a Reference) -> BoxFuture<'a, Result<Option<Vec<u8>>>>;

    /// Persist `bytes` as `reference`; a release already stored is left as it
    /// was.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot persist them.
    fn put<'a>(&'a self, reference: &'a Reference, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>>;

    /// Where the store would hold `reference`, as a refusal names it.
    fn describe(&self, reference: &Reference) -> String;
}
