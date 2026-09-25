//! Guest sources: where a declared guest's component bytes come from, and
//! how they become a guest.
//!
//! A [`Source`] is one `[[guest]]` entry resolved for loading: the identity
//! it registers under, the [`SourceSpec`] naming its bytes, the digest the
//! bytes must hash to, and whether pre-compiled bytes are admitted. A boot
//! guest loads through [`Source::load`]; the guest loader reads an on-demand
//! guest through [`Source::read`] (or its registry, for a package), verifies
//! the bytes through [`Source::verified`], and admits them through the
//! runtime. The checks are one body either way, so the two paths cannot
//! drift.

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use serde::Deserialize;
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::artifact::component;
use crate::digest::Digest;
use crate::registry::GuestId;

/// A compiled component paired with the identity to register it under.
pub struct LoadedGuest {
    /// The identity the guest registers under.
    pub id: GuestId,
    /// The compiled component.
    pub component: Component,
    /// The digest of the bytes it was loaded from.
    pub digest: Digest,
}

/// Where a guest's component bytes come from.
///
/// Modelled as an externally tagged enum so TOML's `source.path = "..."` and
/// `source.package = "..."` each select a variant.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceSpec {
    /// A local component file: raw `.wasm`, or `omnia compile` output. A
    /// manifest loaded from a file resolves relative paths against the
    /// manifest's directory; a relative path set programmatically resolves
    /// against the process working directory.
    Path(PathBuf),
    /// Component bytes embedded in the host binary (typically an
    /// `include_bytes!` blob). TOML cannot express this variant; it is set
    /// through the `runtime!` macro or the programmatic guest-entry API.
    #[serde(skip)]
    Bytes(Cow<'static, [u8]>),
    /// An exact `namespace:name@version` package reference, fetched on first
    /// load from the registry the deployment's `[registries]` routes it to.
    /// Always loads on demand.
    Package(String),
}

impl SourceSpec {
    /// A package source from its exact `namespace:name@version` reference.
    #[must_use]
    pub fn package(reference: impl Into<String>) -> Self {
        Self::Package(reference.into())
    }
}

// Manual: the derived impl would dump the embedded component bytes.
impl fmt::Debug for SourceSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Self::Bytes(bytes) => write!(f, "Bytes({} bytes)", bytes.len()),
            Self::Package(package) => f.debug_tuple("Package").field(package).finish(),
        }
    }
}

impl fmt::Display for SourceSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => write!(f, "{}", path.display()),
            Self::Bytes(bytes) => write!(f, "{} embedded bytes", bytes.len()),
            Self::Package(package) => write!(f, "package `{package}`"),
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

/// One declared guest, resolved for loading: its identity, where its bytes
/// come from, and what the bytes must satisfy before they become a guest.
#[derive(Clone, Debug)]
pub struct Source {
    id: GuestId,
    spec: SourceSpec,
    digest: Option<Digest>,
    wasm_only: bool,
}

impl Source {
    /// A source registering `spec`'s bytes under `id`, unpinned and admitting
    /// either artifact format.
    #[must_use]
    pub fn new(id: impl Into<GuestId>, spec: impl Into<SourceSpec>) -> Self {
        Self {
            id: id.into(),
            spec: spec.into(),
            digest: None,
            wasm_only: false,
        }
    }

    /// Require the bytes to hash to `digest`; a source that resolves to
    /// anything else is refused.
    #[must_use]
    pub const fn pinned(mut self, digest: Digest) -> Self {
        self.digest = Some(digest);
        self
    }

    /// Admit raw wasm alone: a pre-compiled artifact is refused however it
    /// hashes.
    ///
    /// Pre-compiled bytes are deserialized as native code, so a source the
    /// deployment declares from input it does not author — a path and a pin
    /// read from the same untrusted place as the bytes — should carry this.
    #[must_use]
    pub const fn wasm_only(mut self) -> Self {
        self.wasm_only = true;
        self
    }

    /// The identity this source registers under.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Where the bytes come from.
    #[must_use]
    pub const fn spec(&self) -> &SourceSpec {
        &self.spec
    }

    /// The digest the bytes must hash to, if pinned.
    #[must_use]
    pub const fn digest(&self) -> Option<Digest> {
        self.digest
    }

    /// Whether a pre-compiled artifact is refused.
    #[must_use]
    pub const fn is_wasm_only(&self) -> bool {
        self.wasm_only
    }

    /// Read the bytes a [`SourceSpec::Path`] or [`SourceSpec::Bytes`] source
    /// names.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, or the source is a
    /// [`SourceSpec::Package`] — a package is fetched from its registry by
    /// the guest loader, never read here.
    pub async fn read(&self) -> Result<Vec<u8>> {
        match &self.spec {
            SourceSpec::Path(path) => tokio::fs::read(path)
                .await
                .with_context(|| format!("reading `{}` for guest `{}`", path.display(), self.id)),
            SourceSpec::Bytes(bytes) => Ok(bytes.to_vec()),
            SourceSpec::Package(package) => bail!(
                "guest `{}`: package `{package}` is fetched from a registry on demand, not read",
                self.id
            ),
        }
    }

    /// The digest of `bytes`, once they satisfy what this source declares:
    /// they hash to the pin, if any, and are raw wasm where the source admits
    /// raw wasm alone.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes miss the declared digest, or are a
    /// pre-compiled artifact on a [`wasm_only`](Self::wasm_only) source.
    /// Either is a refusal: the same bytes will never pass.
    pub fn verified(&self, bytes: &[u8]) -> Result<Digest> {
        let digest = Digest::of(bytes);
        if let Some(declared) = self.digest {
            ensure!(
                declared == digest,
                "guest `{}` resolved to {digest}, not its declared digest {declared}",
                self.id
            );
        }
        if self.wasm_only && Engine::detect_precompiled(bytes).is_some() {
            bail!("guest `{}` is pre-compiled, but its entry admits raw wasm alone", self.id);
        }
        Ok(digest)
    }

    /// Read, verify, and compile this source into the guest it registers at
    /// boot, recording the digest of the bytes it was loaded from.
    ///
    /// Compilation is CPU-bound, so it runs on a blocking thread — loading
    /// several guests concurrently compiles them in parallel.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes cannot be [read](Self::read), fail
    /// [verification](Self::verified), or do not load as a component.
    pub async fn load(&self, engine: &Engine) -> Result<LoadedGuest> {
        let bytes = self.read().await?;
        let digest = self.verified(&bytes)?;
        let component = component(engine, bytes)
            .await
            .with_context(|| format!("loading guest `{}` from {}", self.id, self.spec))?;
        Ok(LoadedGuest {
            id: self.id.clone(),
            component,
            digest,
        })
    }
}
