//! Guest sources: where a declared guest's component bytes come from, and
//! how they become a guest.
//!
//! A [`Source`] is one `[[guest]]` entry resolved for loading: the identity
//! it registers under, the [`SourceSpec`] naming its bytes, and the digest
//! the bytes must hash to. Embedded bytes load at boot through
//! [`Source::load`]; a path or package loads at first use — read through
//! [`Source::read`], or fetched from the runtime's package source — verified
//! through [`Source::verified`] and admitted through the runtime. The checks
//! are one body either way, so the two paths cannot drift.
//!
//! What verification yields is a [`Verified`]: the bytes with their digest,
//! and the only thing the runtime will load a component from. A
//! pre-compiled artifact is native code, so holding a `Verified` is the proof
//! `Component::deserialize` asks of its caller. Where native code is admitted
//! follows from the source kind alone: embedded bytes were in the process
//! before any guest ran, a path is read while guests run and so needs the
//! deployment's pin, and a package admits raw wasm alone.

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use futures::future::BoxFuture;
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
    /// A local component file, read at first use: raw `.wasm`, or `omnia
    /// compile` output when the entry pins its `digest`. A manifest loaded
    /// from a file resolves relative paths against the manifest's directory;
    /// a relative path set programmatically resolves against the process
    /// working directory.
    Path(PathBuf),
    /// Component bytes the process already holds (typically an
    /// `include_bytes!` blob), loaded at boot in either format. TOML cannot
    /// express this variant; it is set through the `runtime!` macro or the
    /// programmatic guest-entry API.
    #[serde(skip)]
    Bytes(Cow<'static, [u8]>),
    /// An exact `namespace:name@version` package reference, fetched at first
    /// use from the registry the deployment's `[registries]` routes it to;
    /// raw wasm alone.
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
}

impl Source {
    /// A source registering `spec`'s bytes under `id`, unpinned.
    #[must_use]
    pub fn new(id: impl Into<GuestId>, spec: impl Into<SourceSpec>) -> Self {
        Self {
            id: id.into(),
            spec: spec.into(),
            digest: None,
        }
    }

    /// Require the bytes to hash to `digest`; a source that resolves to
    /// anything else is refused.
    #[must_use]
    pub const fn pinned(mut self, digest: Digest) -> Self {
        self.digest = Some(digest);
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

    /// Read the bytes a [`SourceSpec::Path`] or [`SourceSpec::Bytes`] source
    /// names.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, or the source is a
    /// [`SourceSpec::Package`] — a package is fetched by the runtime's
    /// package source, never read here.
    pub async fn read(&self) -> Result<Vec<u8>> {
        match &self.spec {
            SourceSpec::Path(path) => tokio::fs::read(path)
                .await
                .with_context(|| format!("reading `{}` for guest `{}`", path.display(), self.id)),
            SourceSpec::Bytes(bytes) => Ok(bytes.to_vec()),
            SourceSpec::Package(package) => bail!(
                "guest `{}`: package `{package}` is fetched from a registry, not read",
                self.id
            ),
        }
    }

    /// `bytes` admitted for loading, once they satisfy what this source
    /// declares: they hash to the pin, if any, and a pre-compiled artifact
    /// comes from embedded bytes or a pinned path.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes miss the declared digest, or are a
    /// pre-compiled artifact from a package or an unpinned path. Each is a
    /// refusal: the same bytes will never pass.
    pub fn verified(&self, bytes: Vec<u8>) -> Result<Verified> {
        let digest = Digest::checked(&bytes, self.digest, format_args!("guest `{}`", self.id))?;
        if Engine::detect_precompiled(&bytes).is_some() {
            match &self.spec {
                SourceSpec::Package(_) => bail!(
                    "guest `{}` is pre-compiled, but a package admits raw wasm alone",
                    self.id
                ),
                // a path is read at first use, while guests run: only the pin vouches for it
                SourceSpec::Path(_) if self.digest.is_none() => bail!(
                    "guest `{}` is pre-compiled and is read from unpinned {} while guests run: \
                     pin its `digest`",
                    self.id,
                    self.spec
                ),
                SourceSpec::Path(_) | SourceSpec::Bytes(_) => {}
            }
        }
        Ok(Verified { bytes, digest })
    }

    /// Read, verify, and compile this source into the guest it registers,
    /// recording the digest of the bytes it was loaded from.
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
        let verified = self.verified(bytes)?;
        let digest = verified.digest();
        let component = component(engine, verified)
            .await
            .with_context(|| format!("loading guest `{}` from {}", self.id, self.spec))?;
        Ok(LoadedGuest {
            id: self.id.clone(),
            component,
            digest,
        })
    }
}

/// Registry acquisition policy — how a [`SourceSpec::Package`] source is
/// fetched.
pub trait RegistrySource: Send + Sync + 'static {
    /// Produce the raw component bytes for the exact `package` reference —
    /// from `endpoint` when the load names one and the deployment's routing
    /// does not claim the package's namespace, else from that routing —
    /// split by remedy: [`AcquireError::Refused`] for an authoritative "no"
    /// (nothing routes it, a routed namespace named another registry, the
    /// registry has no such release), never for a source failure a retry
    /// might clear ([`AcquireError::Unavailable`]).
    fn acquire<'a>(
        &'a self, package: &'a str, endpoint: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>>;
}

/// Why a [`RegistrySource`] could not produce a package's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcquireError {
    /// Nothing routes the package, or the registry has no such release; a
    /// retry cannot succeed.
    Refused(String),
    /// The registry could not produce the bytes; a retry may succeed.
    Unavailable(String),
}

impl fmt::Display for AcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(detail) => write!(f, "refused: {detail}"),
            Self::Unavailable(detail) => write!(f, "unavailable: {detail}"),
        }
    }
}

impl std::error::Error for AcquireError {}

/// Component bytes admitted for loading, with their digest.
///
/// One exists only through [`Source::verified`] (the declared policy),
/// [`Verified::wasm`] (raw wasm alone), or the `unsafe`
/// [`Verified::trusted`]: holding one is the proof `Component::deserialize`
/// asks its caller for, so it is the only thing the runtime loads a component
/// from.
pub struct Verified {
    bytes: Vec<u8>,
    digest: Digest,
}

impl Verified {
    /// Raw wasm from a caller; a pre-compiled artifact is refused, since
    /// nothing here attests where a caller's bytes came from.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are a pre-compiled artifact.
    pub fn wasm(bytes: Vec<u8>) -> Result<Self> {
        ensure!(
            Engine::detect_precompiled(&bytes).is_none(),
            "the bytes are a pre-compiled artifact; `Verified::wasm` admits raw wasm alone"
        );
        Ok(Self {
            digest: Digest::of(&bytes),
            bytes,
        })
    }

    /// Bytes in either format, on the caller's word.
    ///
    /// # Safety
    ///
    /// If pre-compiled, `bytes` must be the unmodified output of `omnia
    /// compile` from a build pipeline the caller trusts: they are native code,
    /// and wasmtime's settings check is compatibility, not authenticity.
    #[must_use]
    pub unsafe fn trusted(bytes: Vec<u8>) -> Self {
        Self {
            digest: Digest::of(&bytes),
            bytes,
        }
    }

    /// The content digest of the bytes.
    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

// Manual: the derived impl would dump the component bytes.
impl fmt::Debug for Verified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Verified({} bytes, {})", self.bytes.len(), self.digest)
    }
}

// The fixture is compiled by wasmtime itself, so these run only with the
// compiler linked in.
#[cfg(all(test, feature = "jit"))]
mod tests {
    use super::*;

    // The smallest component: header and version alone.
    const EMPTY_COMPONENT: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

    fn precompiled() -> Vec<u8> {
        Engine::default().precompile_component(&EMPTY_COMPONENT).expect("compiling the fixture")
    }

    #[test]
    fn verified_wasm_refuses_precompiled() {
        let error = Verified::wasm(precompiled()).expect_err("native code is refused");
        assert!(error.to_string().contains("raw wasm alone"), "{error}");

        let raw = Verified::wasm(EMPTY_COMPONENT.to_vec()).expect("raw wasm passes");
        assert_eq!(raw.digest(), Digest::of(&EMPTY_COMPONENT));
    }

    // Pre-compiled bytes pass from embedded bytes or a pinned path, never
    // from an unpinned path or a package; raw wasm passes every source.
    #[test]
    fn verified_matrix() {
        let native = precompiled();
        let pin = Digest::of(&native);
        let path = || Source::new("plugin", "plugin.bin");
        let package = || Source::new("plugin", SourceSpec::package("acme:plugin@1.0.0"));

        for (source, admitted) in [
            (path(), false),
            (path().pinned(pin), true),
            (Source::new("plugin", native.clone()), true),
            (package().pinned(pin), false),
        ] {
            let outcome = source.verified(native.clone());
            assert_eq!(outcome.is_ok(), admitted, "{source:?}: {:?}", outcome.err());
        }
        let refused = path().verified(native.clone()).expect_err("an unpinned path");
        assert!(refused.to_string().contains("unpinned"), "{refused}");
        let refused = package().pinned(pin).verified(native).expect_err("a package");
        assert!(refused.to_string().contains("a package admits raw wasm alone"), "{refused}");

        let raw_pin = Digest::of(&EMPTY_COMPONENT);
        for source in [
            path(),
            path().pinned(raw_pin),
            Source::new("plugin", EMPTY_COMPONENT.to_vec()),
            package().pinned(raw_pin),
        ] {
            source.verified(EMPTY_COMPONENT.to_vec()).expect("raw wasm passes every source");
        }
    }

    #[test]
    fn pin_mismatch() {
        let error = Source::new("plugin", "plugin.wasm")
            .pinned(Digest::of(b"other bytes"))
            .verified(EMPTY_COMPONENT.to_vec())
            .expect_err("the pin misses");
        assert!(error.to_string().contains("not its declared digest"), "{error}");
    }
}
