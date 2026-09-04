//! # Guest acquisition
//!
//! Where a guest's component bytes come from. The deployment manifest's
//! `source` field selects a kind per guest: a local `.wasm` / pre-compiled
//! `.bin` path, or component bytes embedded in the host binary. OCI would
//! land as another kind.

use std::borrow::Cow;
use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};
use omnia_core::wasmtime::Engine;
use omnia_core::{ELF_MAGIC, GuestArtifact, GuestId, LoadedGuest, is_precompiled};

/// Whether a deployment build may load pre-compiled (native) artifacts.
///
/// Crate-internal on purpose: the only door to `Trust` is an `unsafe`
/// call site ([`DeploymentBuilder::build_trusted`](crate::DeploymentBuilder::build_trusted)
/// or [`GuestArtifact::precompiled`](omnia_core::GuestArtifact::precompiled)).
// `pub` in a private module: crate-internal, never re-exported.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArtifactPolicy {
    /// Only raw wasm components load; a pre-compiled artifact is rejected.
    Reject,
    /// Pre-compiled artifacts load via native deserialization; the caller has
    /// attested trust through an `unsafe` API.
    Trust,
}

/// A guest loaded from a local `.wasm` (or pre-compiled `.bin`) file, or from
/// component bytes embedded in the host binary.
pub struct Source {
    id: GuestId,
    kind: SourceKind,
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
        }
    }

    /// Create an embedded source registering `bytes` under an explicit
    /// identity (typically an `include_bytes!` blob).
    #[must_use]
    pub fn embedded(id: GuestId, bytes: impl Into<Cow<'static, [u8]>>) -> Self {
        Self {
            id,
            kind: SourceKind::Bytes(bytes.into()),
        }
    }

    /// Returns the identity this source registers under.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Load the component this source registers, under the build's artifact
    /// policy.
    ///
    /// Async so a future source kind (an OCI pull) fits the same signature.
    /// Compilation is CPU-bound, so it runs on a blocking thread — loading
    /// several guests concurrently compiles them in parallel.
    pub(crate) async fn load(
        &self, engine: &Engine, policy: ArtifactPolicy,
    ) -> Result<LoadedGuest> {
        let artifact = match &self.kind {
            SourceKind::Path(path) => {
                if is_precompiled(path)? {
                    ensure!(
                        policy == ArtifactPolicy::Trust,
                        "{} is a pre-compiled (native) artifact, which this build rejects; load \
                         trusted pre-compiled artifacts through `DeploymentBuilder`'s unsafe \
                         `build_trusted`",
                        path.display()
                    );
                    // SAFETY: `policy == Trust` is only reachable through an
                    // `unsafe` build call whose caller attested every
                    // pre-compiled path names unmodified trusted wasmtime
                    // output — the contract `precompiled_file` requires.
                    unsafe { GuestArtifact::precompiled_file(path.clone()) }
                } else {
                    GuestArtifact::wasm(
                        std::fs::read(path)
                            .with_context(|| format!("loading guest from {}", path.display()))?,
                    )
                }
            }
            SourceKind::Bytes(bytes) => {
                if bytes.get(..ELF_MAGIC.len()) == Some(&ELF_MAGIC) {
                    ensure!(
                        policy == ArtifactPolicy::Trust,
                        "the embedded bytes are a pre-compiled (native) artifact, which this \
                         build rejects; load trusted pre-compiled artifacts through \
                         `DeploymentBuilder`'s unsafe `build_trusted`"
                    );
                    // SAFETY: `policy == Trust` is only reachable through an
                    // `unsafe` build call whose caller attested every
                    // pre-compiled artifact is unmodified trusted wasmtime
                    // output — the contract `precompiled` requires.
                    unsafe { GuestArtifact::precompiled(bytes.to_vec()) }
                } else {
                    GuestArtifact::wasm(bytes.to_vec())
                }
            }
        };
        let component = artifact.load(engine).await.with_context(|| match &self.kind {
            SourceKind::Path(path) => format!("loading guest from {}", path.display()),
            SourceKind::Bytes(_) => format!("loading embedded guest `{}`", self.id),
        })?;
        Ok(LoadedGuest {
            id: self.id.clone(),
            component,
        })
    }
}
