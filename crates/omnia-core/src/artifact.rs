//! Guest component artifacts: the bytes a guest loads from, in either format.

use anyhow::{Context as _, Result};
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::digest::Digest;
use crate::registry::GuestId;

// Magic of a wasmtime-serialized (native ELF) artifact: what `omnia compile`
// emits, told from raw wasm by its leading bytes.
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

// Appended to every pre-compiled deserialization failure: the usual cause is a
// compile-affecting settings mismatch, not corruption.
const SETTINGS_HINT: &str = "the artifact must be built with the same compile-affecting settings \
                             used by `omnia compile` (MAX_FUEL, BRANCH_HINTING, \
                             MEMORY_RESERVATION, MEMORY_GUARD_SIZE)";

/// A compiled component paired with the identity to register it under.
pub struct LoadedGuest {
    /// The identity the component is registered under.
    pub id: GuestId,
    /// The compiled component.
    pub component: Component,
    /// The digest of the bytes it was loaded from.
    pub digest: Digest,
}

/// Component bytes for registration
/// ([`Runtime::register`](crate::Runtime::register)): a raw wasm component,
/// compiled at load, or `omnia compile` output, deserialized at load.
///
/// The leading bytes decide which. Verification (digest, signature,
/// provenance) is deployment policy and happens before the runtime sees the
/// bytes: a pre-compiled artifact is native code, loaded on the deployment's
/// word that it is what `omnia compile` produced.
pub struct GuestArtifact(Vec<u8>);

impl GuestArtifact {
    /// Component bytes in either format.
    #[must_use]
    pub const fn bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Load the artifact into a [`Component`] on a blocking thread
    /// (deserialization and compilation are CPU-bound).
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization or compilation fails, raw wasm is
    /// given without the `jit` feature, or the blocking load task panics.
    pub async fn load(self, engine: &Engine) -> Result<Component> {
        let engine = engine.clone();
        tokio::task::spawn_blocking(move || {
            let component = if self.0.starts_with(&ELF_MAGIC) {
                // SAFETY: deployment inputs are trusted operator inputs
                // (docs/security-model.md): these bytes are the artifact the
                // deployment declared, and a pre-compiled one is `omnia
                // compile` output by that declaration — the contract
                // `Component::deserialize` requires.
                unsafe { Component::deserialize(&engine, &self.0) }
                    .map_err(anyhow::Error::from)
                    .with_context(|| {
                        format!("deserializing pre-compiled component: {SETTINGS_HINT}")
                    })?
            } else {
                #[cfg(feature = "jit")]
                {
                    Component::new(&engine, &self.0)
                        .map_err(anyhow::Error::from)
                        .context("compiling component")?
                }
                #[cfg(not(feature = "jit"))]
                anyhow::bail!(
                    "compiling raw wasm requires the `jit` feature; pre-compile the component \
                     with `omnia compile` instead"
                )
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
