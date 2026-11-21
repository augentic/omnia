//! `omnia:plugins`, the guests a guest late-binds into its deployment.
//!
//! A guest names a [`Source`]:
//!
//! - a guest the deployment declares
//! - a component file beneath one of its read-only mounts
//! - a release its package store or one of its registries holds
//!
//! [`load`] has the host acquire, verify, and admit the bytes the source
//! names, or return the guest already loaded under its name. Component bytes
//! never cross the interface, and every source resolves inside what the
//! deployment grants.

use std::fmt;
use std::path::Path;

use crate::Result;
pub use crate::bindings::imports::omnia::plugins::types::{
    Conflict, File, Plugin, Release, Source,
};
pub use crate::bindings::imports::omnia::plugins::*;

/// Load the guest `source` names and return it.
///
/// Loading the same bytes under a name again returns the guest as it stands.
///
/// # Errors
///
/// Returns an [`Error`](crate::Error) when the source names nothing, the
/// deployment does not grant the load, the bytes are not a component it can
/// run, or the bytes could not be produced. A [`Conflict`] is the inner
/// `Err`: the bytes missed the source's pin, or a guest is loaded under the
/// name from other bytes.
pub async fn load(source: Source) -> Result<Result<Plugin, Conflict>> {
    Ok(loader::load(source).await?)
}

impl Source {
    /// The name a guest loaded from this source loads under.
    ///
    /// # Examples
    ///
    /// ```
    /// use omnia_sdk::plugins::{File, Release, Source};
    ///
    /// assert_eq!(Source::Declared("intent".into()).name(), "intent");
    /// let file = Source::Path(File {
    ///     path: "./adapters/intent.wasm".into(),
    ///     pin: None,
    /// });
    /// assert_eq!(file.name(), "intent");
    /// let release = Source::Release(Release {
    ///     reference: "emery:intent@1.0.0".into(),
    ///     registry: None,
    ///     pin: None,
    /// });
    /// assert_eq!(release.name(), "emery:intent");
    /// ```
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Declared(name) => name,
            Self::Path(file) => Path::new(&file.path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(&file.path),
            Self::Release(release) => release
                .reference
                .split_once('@')
                .map_or(release.reference.as_str(), |(name, _)| name),
        }
    }
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.current {
            Some(current) if current.digest != self.found => write!(
                f,
                "`{}` is loaded from {}, not {}",
                current.name, current.digest, self.found
            ),
            _ => write!(f, "the bytes hash to {}, not the pinned digest", self.found),
        }
    }
}

impl std::error::Error for Conflict {}
