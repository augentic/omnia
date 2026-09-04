//! Guest component artifacts: raw wasm and pre-compiled native images.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::registry::GuestId;

/// Magic of a wasmtime-serialized (native ELF) artifact, sniffed from content.
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

// Appended to every pre-compiled deserialization failure: the usual cause is a
// compile-affecting settings mismatch, not corruption.
const SETTINGS_HINT: &str = "the artifact must be built with the same compile-affecting settings \
                             used by `omnia compile` (MAX_FUEL, BRANCH_HINTING, \
                             MEMORY_RESERVATION, MEMORY_GUARD_SIZE)";

/// Raw component bytes paired with the identity to register them under.
pub struct LoadedGuest {
    /// The identity the component is registered under.
    pub id: GuestId,
    /// The compiled component.
    pub component: Component,
}

/// Component bytes for dynamic registration
/// ([`Runtime::register`](crate::Runtime::register)).
///
/// [`wasm`](Self::wasm) is safe (the bytes are validated and compiled inside
/// the sandbox). [`precompiled`](Self::precompiled) and
/// [`precompiled_file`](Self::precompiled_file) are `unsafe` (the bytes are
/// native code the caller attests came from a trusted build pipeline).
/// Verification (digest, signature, provenance) is deployment policy and
/// happens before the runtime sees the bytes.
pub struct GuestArtifact(ArtifactKind);

enum ArtifactKind {
    /// Raw component wasm, JIT-compiled at registration. Without `jit` the
    /// bytes are never read — loading bails before compilation.
    Wasm(Vec<u8>),
    /// A settings-matched pre-compiled artifact (`omnia compile` output),
    /// loaded via native deserialization with no runtime codegen.
    Precompiled(Vec<u8>),
    /// A settings-matched pre-compiled artifact on disk (`omnia compile`
    /// output), loaded via native file deserialization with no runtime codegen.
    PrecompiledFile(PathBuf),
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

    /// A settings-matched pre-compiled artifact (`omnia compile` output) on
    /// disk, loaded via file deserialization with no runtime codegen.
    ///
    /// # Safety
    ///
    /// The file at `path` must be the unmodified output of wasmtime component
    /// serialization (`omnia compile` / [`Component::serialize`]) produced by
    /// a trusted build pipeline. A pre-compiled artifact is native code:
    /// wasmtime's compatibility check (rejecting mismatched compile-affecting
    /// settings) is *not* an authenticity check, and tampered bytes can
    /// execute arbitrary code with host privileges.
    #[must_use]
    pub const unsafe fn precompiled_file(path: PathBuf) -> Self {
        Self(ArtifactKind::PrecompiledFile(path))
    }

    /// Load the artifact into a [`Component`] on a blocking thread
    /// (deserialization and compilation are CPU-bound).
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are a native artifact on the wasm path,
    /// compilation or deserialization fails, or the blocking load task panics.
    pub async fn load(self, engine: &Engine) -> Result<Component> {
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
                        .with_context(|| {
                            format!("deserializing pre-compiled guest: {SETTINGS_HINT}")
                        })?
                }
                ArtifactKind::PrecompiledFile(path) => {
                    // SAFETY: the `GuestArtifact::precompiled_file`
                    // constructor is `unsafe`; its caller attested this file
                    // is unmodified trusted wasmtime output, which is exactly
                    // the contract `Component::deserialize_file` requires.
                    unsafe { Component::deserialize_file(&engine, &path) }
                        .map_err(anyhow::Error::from)
                        .with_context(|| {
                            format!(
                                "deserializing pre-compiled component {}: {SETTINGS_HINT}",
                                path.display()
                            )
                        })?
                }
                ArtifactKind::Wasm(bytes) => {
                    ensure!(
                        bytes.get(..ELF_MAGIC.len()) != Some(&ELF_MAGIC),
                        "the bytes are a pre-compiled (native) artifact; GuestArtifact::wasm \
                         only accepts raw wasm"
                    );
                    #[cfg(feature = "jit")]
                    {
                        Component::new(&engine, &bytes)
                            .map_err(anyhow::Error::from)
                            .context("compiling guest component")?
                    }
                    #[cfg(not(feature = "jit"))]
                    anyhow::bail!(
                        "registering raw wasm requires the `jit` feature; pre-compile with `omnia \
                         compile` and register the artifact instead"
                    )
                }
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

/// Whether `path` holds a wasmtime-serialized (native ELF) artifact, sniffed
/// from the leading magic bytes.
pub fn is_precompiled(path: &Path) -> Result<bool> {
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
