//! # Guest acquisition at boot
//!
//! Where a boot guest's component bytes come from. The deployment manifest's
//! `source` field selects a kind per guest: a local path — a raw `.wasm` or
//! `omnia compile` output — or component bytes embedded in the host binary. A
//! package source is fetched by the guest loader on first load, never here.

use std::borrow::Cow;
use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};
use omnia_core::wasmtime::Engine;
use omnia_core::{Digest, GuestArtifact, GuestId, LoadedGuest};

/// A guest loaded from a local component file, or from component bytes
/// embedded in the host binary.
pub struct Source {
    id: GuestId,
    kind: SourceKind,
    digest: Option<Digest>,
}

/// Where the component bytes live.
enum SourceKind {
    Path(PathBuf),
    Bytes(Cow<'static, [u8]>),
}

impl Source {
    /// Create a file source registering under an explicit identity.
    #[must_use]
    pub fn with_id(id: GuestId, path: impl Into<PathBuf>) -> Self {
        Self {
            id,
            kind: SourceKind::Path(path.into()),
            digest: None,
        }
    }

    /// Create an embedded source registering `bytes` under an explicit
    /// identity (typically an `include_bytes!` blob).
    #[must_use]
    pub fn embedded(id: GuestId, bytes: impl Into<Cow<'static, [u8]>>) -> Self {
        Self {
            id,
            kind: SourceKind::Bytes(bytes.into()),
            digest: None,
        }
    }

    /// Require the bytes to hash to `digest`; a source that resolves to
    /// anything else fails to load.
    #[must_use]
    pub const fn pinned(mut self, digest: Digest) -> Self {
        self.digest = Some(digest);
        self
    }

    /// Returns the identity this source registers under.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Load the component this source registers, recording the digest of the
    /// bytes it was loaded from.
    ///
    /// Compilation is CPU-bound, so it runs on a blocking thread — loading
    /// several guests concurrently compiles them in parallel.
    pub(crate) async fn load(&self, engine: &Engine) -> Result<LoadedGuest> {
        let bytes = match &self.kind {
            SourceKind::Path(path) => std::fs::read(path)
                .with_context(|| format!("loading guest from {}", path.display()))?,
            SourceKind::Bytes(bytes) => bytes.to_vec(),
        };
        let digest = self.verified(&bytes)?;
        let component =
            GuestArtifact::bytes(bytes).load(engine).await.with_context(|| match &self.kind {
                SourceKind::Path(path) => format!("loading guest from {}", path.display()),
                SourceKind::Bytes(_) => format!("loading embedded guest `{}`", self.id),
            })?;
        Ok(LoadedGuest {
            id: self.id.clone(),
            component,
            digest,
        })
    }

    /// The digest of `bytes`, once they match the declared pin, if any.
    fn verified(&self, bytes: &[u8]) -> Result<Digest> {
        let digest = Digest::of(bytes);
        if let Some(declared) = self.digest {
            ensure!(
                declared == digest,
                "guest `{}` resolved to {digest}, not its declared digest {declared}",
                self.id
            );
        }
        Ok(digest)
    }
}
