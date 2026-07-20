//! # Guest acquisition
//!
//! Where a guest's component bytes come from. The deployment manifest's
//! `source` field selects a kind per guest. Today only [`Source`] (a local
//! `.wasm` / pre-compiled `.bin` path) exists; OCI would land as another kind.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::registry::GuestId;

// Wasmtime-serialized artifacts are native ELF images; raw components carry
// the `\0asm` wasm magic. The distinction gates the unsafe deserialization
// path, so it is sniffed from content, never inferred from a file extension.
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// Whether a deployment build may load pre-compiled (native) artifacts.
///
/// Crate-internal on purpose: the only door to `Trust` is an `unsafe`
/// call site ([`DeploymentBuilder::build`](crate::DeploymentBuilder) in the
/// `Precompiled` typestate, or [`GuestArtifact::precompiled`]).
// `pub` in a private module: crate-internal, never re-exported.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArtifactPolicy {
    /// Only raw wasm components load; a pre-compiled artifact is rejected.
    Reject,
    /// Pre-compiled artifacts load via native deserialization; the caller has
    /// attested trust through an `unsafe` API.
    Trust,
}

/// Raw component bytes paired with the identity to register them under.
pub struct LoadedGuest {
    /// The identity the component is registered under.
    pub id: GuestId,
    /// The compiled component.
    pub component: Component,
}

/// A guest loaded from a local `.wasm` (or pre-compiled `.bin`) file.
///
/// `omnia run <guest>.wasm` is the one-guest shorthand: load it, derive its
/// identity from the file stem, and register it as the default guest.
pub struct Source {
    id: GuestId,
    path: PathBuf,
}

impl Source {
    /// Create a file source, deriving the identity from the file stem
    /// (`./guests/echo.wasm` -> `echo`).
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let id = id_from_path(&path);
        Self { id, path }
    }

    /// Create a file source registering under an explicit identity.
    #[must_use]
    pub fn with_id(id: GuestId, path: impl Into<PathBuf>) -> Self {
        Self {
            id,
            path: path.into(),
        }
    }

    /// Returns the identity this source registers under.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Load the component(s) this source registers, under the build's
    /// artifact policy.
    ///
    /// Async so a future source kind (an OCI pull) fits the same signature.
    /// Compilation is CPU-bound, so it runs on a blocking thread — loading
    /// several guests concurrently compiles them in parallel.
    pub(crate) async fn load(
        &self, engine: &Engine, policy: ArtifactPolicy,
    ) -> Result<Vec<LoadedGuest>> {
        let engine = engine.clone();
        let path = self.path.clone();
        let component = tokio::task::spawn_blocking(move || {
            load_component(&engine, &path, policy)
                .with_context(|| format!("loading guest from {}", path.display()))
        })
        .await
        .context("guest load task panicked")??;
        Ok(vec![LoadedGuest {
            id: self.id.clone(),
            component,
        }])
    }
}

/// Component bytes for dynamic registration
/// ([`Runtime::register`](crate::Runtime::register)).
///
/// The two constructors carry different trust: [`wasm`](Self::wasm) is safe
/// (the bytes are validated and compiled inside the sandbox), while
/// [`precompiled`](Self::precompiled) is `unsafe` (the bytes are native code
/// the caller attests came from a trusted build pipeline). Verification
/// (digest, signature, provenance) is deployment policy and happens before
/// the runtime sees the bytes.
pub struct GuestArtifact(ArtifactKind);

enum ArtifactKind {
    /// Raw component wasm, JIT-compiled at registration. Without `jit` the
    /// bytes are never read — loading bails before compilation.
    #[cfg_attr(not(feature = "jit"), allow(dead_code))]
    Wasm(Vec<u8>),
    /// A settings-matched pre-compiled artifact (`omnia compile` output),
    /// loaded via native deserialization with no runtime codegen.
    Precompiled(Vec<u8>),
}

impl GuestArtifact {
    /// Raw component wasm, JIT-compiled at registration (requires the `jit`
    /// feature). Validated and compiled by wasmtime; safe to accept from
    /// less-trusted sources.
    #[must_use]
    pub const fn wasm(bytes: Vec<u8>) -> Self {
        Self(ArtifactKind::Wasm(bytes))
    }

    /// A settings-matched pre-compiled artifact (`omnia compile` output),
    /// loaded via deserialization with no runtime codegen.
    ///
    /// # Safety
    ///
    /// `bytes` must be the unmodified output of wasmtime component
    /// serialization (`omnia compile` / [`Component::serialize`]) produced by
    /// a trusted build pipeline. A pre-compiled artifact is native code:
    /// wasmtime's compatibility check (rejecting mismatched compile-affecting
    /// settings) is *not* an authenticity check, and tampered bytes can
    /// execute arbitrary code with host privileges.
    #[must_use]
    pub const unsafe fn precompiled(bytes: Vec<u8>) -> Self {
        Self(ArtifactKind::Precompiled(bytes))
    }

    /// Load the artifact into a [`Component`] on a blocking thread
    /// (deserialization and compilation are CPU-bound).
    pub(crate) async fn load(self, engine: &Engine) -> Result<Component> {
        let engine = engine.clone();
        tokio::task::spawn_blocking(move || {
            let component = match self.0 {
                ArtifactKind::Precompiled(bytes) => {
                    // SAFETY: the `GuestArtifact::precompiled` constructor is
                    // `unsafe`; its caller attested these bytes are unmodified
                    // trusted wasmtime output, which is exactly the contract
                    // `Component::deserialize` requires.
                    unsafe { Component::deserialize(&engine, &bytes) }
                        .map_err(anyhow::Error::from)
                        .context(
                            "deserializing pre-compiled guest: the artifact must be built with the \
                     same compile-affecting settings used by `omnia compile` (MAX_FUEL, \
                     BRANCH_HINTING, MEMORY_RESERVATION, MEMORY_GUARD_SIZE)",
                        )?
                }
                #[cfg(feature = "jit")]
                ArtifactKind::Wasm(bytes) => Component::new(&engine, &bytes)
                    .map_err(anyhow::Error::from)
                    .context("compiling guest component")?,
                #[cfg(not(feature = "jit"))]
                ArtifactKind::Wasm(_) => anyhow::bail!(
                    "registering raw wasm requires the `jit` feature; pre-compile with `omnia \
                     compile` and register the artifact instead"
                ),
            };
            // Build the copy-on-write heap image now rather than lazily on the
            // first instantiation, moving that one-time cost off the first call.
            component.initialize_copy_on_write_image()?;
            Ok(component)
        })
        .await
        .context("guest load task panicked")?
    }
}

/// Derive an opaque identity from a file path's stem, falling back to `default`
/// when the path has no usable stem.
fn id_from_path(path: &Path) -> GuestId {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("default");
    GuestId::from(stem)
}

/// Load a component from a file: raw wasm compiles (under `jit`); a
/// pre-compiled artifact deserializes only when `policy` trusts it.
fn load_component(engine: &Engine, wasm: &Path, policy: ArtifactPolicy) -> Result<Component> {
    let component = if is_precompiled(wasm)? {
        ensure!(
            policy == ArtifactPolicy::Trust,
            "{} is a pre-compiled (native) artifact, which this build rejects; load trusted \
             pre-compiled artifacts through the `DeploymentBuilder::precompiled()` typestate's \
             unsafe `build`",
            wasm.display()
        );
        // SAFETY: `policy == Trust` is only reachable through an `unsafe`
        // build call whose caller attested every pre-compiled path names
        // unmodified trusted wasmtime output — the contract
        // `Component::deserialize_file` requires.
        unsafe { Component::deserialize_file(engine, wasm) }
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!(
                    "deserializing pre-compiled component {}: the artifact must be built with \
                     the same compile-affecting settings used by `omnia compile` (MAX_FUEL, \
                     BRANCH_HINTING, MEMORY_RESERVATION, MEMORY_GUARD_SIZE)",
                    wasm.display()
                )
            })?
    } else {
        compile_wasm(engine, wasm)?
    };

    // Build the copy-on-write heap image now (startup) rather than lazily on the
    // first instantiation, moving that one-time cost off the first request.
    component.initialize_copy_on_write_image()?;
    Ok(component)
}

/// Compile a raw wasm component from a file.
#[cfg(feature = "jit")]
fn compile_wasm(engine: &Engine, wasm: &Path) -> Result<Component> {
    Component::from_file(engine, wasm)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("compiling component {}", wasm.display()))
}

/// Raw wasm cannot compile without the `jit` feature.
#[cfg(not(feature = "jit"))]
fn compile_wasm(_engine: &Engine, wasm: &Path) -> Result<Component> {
    anyhow::bail!(
        "{} is a raw wasm component and this build has no `jit` feature; pre-compile it with \
         `omnia compile` or rebuild the host with `jit`",
        wasm.display()
    )
}

/// Whether `path` holds a wasmtime-serialized (native ELF) artifact, sniffed
/// from the leading magic bytes.
fn is_precompiled(path: &Path) -> Result<bool> {
    let mut magic = [0u8; 4];
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening component {}", path.display()))?;
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(magic == ELF_MAGIC),
        // Shorter than a magic header: not pre-compiled; let the wasm loader
        // produce its own error.
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error).with_context(|| format!("reading component {}", path.display())),
    }
}
