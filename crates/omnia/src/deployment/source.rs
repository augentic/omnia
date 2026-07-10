//! # Guest acquisition
//!
//! Where a guest's component bytes come from. The deployment manifest's
//! `source` field selects a kind per guest. Today only [`Source`] (a local
//! `.wasm` / pre-compiled `.bin` path) exists; OCI would land as another kind.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::registry::GuestId;

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

    /// Load the component(s) this source registers.
    ///
    /// Async so a future source kind (an OCI pull) fits the same signature.
    /// Compilation is CPU-bound, so it runs on a blocking thread — loading
    /// several guests concurrently compiles them in parallel.
    ///
    /// # Errors
    ///
    /// Returns an error if the component cannot be loaded from the path.
    pub async fn load(&self, engine: &Engine) -> Result<Vec<LoadedGuest>> {
        let engine = engine.clone();
        let path = self.path.clone();
        let component = tokio::task::spawn_blocking(move || {
            load_component(&engine, &path)
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

/// Derive an opaque identity from a file path's stem, falling back to `default`
/// when the path has no usable stem.
fn id_from_path(path: &Path) -> GuestId {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("default");
    GuestId::from(stem)
}

/// Load a component from a file.
fn load_component(engine: &Engine, wasm: &Path) -> Result<Component> {
    // SAFETY: a pre-compiled artifact is rejected (not executed) unless the
    // loading engine matches the compile-affecting settings it was built with.
    let result = unsafe { Component::deserialize_file(engine, wasm) }
        .map_err(anyhow::Error::from)
        .with_context(|| {
            format!(
                "loading component {}: a pre-compiled artifact must be loaded with the same \
                compile-affecting settings used by `omnia compile` (MAX_FUEL, BRANCH_HINTING, \
                MEMORY_RESERVATION, MEMORY_GUARD_SIZE)",
                wasm.display()
            )
        });

    // Fall back to JIT-compiling raw wasm when the feature is enabled.
    #[cfg(feature = "jit")]
    let component =
        result.or_else(|_| Component::from_file(engine, wasm).map_err(anyhow::Error::from))?;

    #[cfg(not(feature = "jit"))]
    let component = result
        .context("if this is a raw wasm32 component, rebuild with the `jit` feature to load it")?;

    // Build the copy-on-write heap image now (startup) rather than lazily on the
    // first instantiation, moving that one-time cost off the first request.
    component.initialize_copy_on_write_image()?;
    Ok(component)
}
