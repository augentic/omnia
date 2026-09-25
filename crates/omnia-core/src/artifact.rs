//! Guest component artifacts: the bytes a guest loads from, in either format.

use anyhow::{Context as _, Result, bail};
use wasmtime::component::Component;
use wasmtime::{Engine, Precompiled};

// Appended to every pre-compiled deserialization failure: the usual cause is a
// compile-affecting settings mismatch, not corruption.
const SETTINGS_HINT: &str = "the artifact must be built with the same compile-affecting settings \
                             used by `omnia compile` (MAX_FUEL, BRANCH_HINTING, \
                             MEMORY_RESERVATION, MEMORY_GUARD_SIZE)";

/// Load component bytes into a [`Component`] on a blocking thread.
///
/// The bytes are a raw wasm component, compiled here, or `omnia compile`
/// output, deserialized here; wasmtime's own detection tells them apart.
/// Verification (digest, signature, provenance) is deployment policy and
/// happens before the runtime sees the bytes: a pre-compiled artifact is
/// native code, loaded on the deployment's word that it is what `omnia
/// compile` produced.
///
/// # Errors
///
/// Returns an error if deserialization or compilation fails, the bytes are a
/// pre-compiled core module rather than a component, raw wasm is given
/// without the `jit` feature, or the blocking load task panics.
pub async fn component(engine: &Engine, bytes: Vec<u8>) -> Result<Component> {
    let engine = engine.clone();
    tokio::task::spawn_blocking(move || {
        let component = load(&engine, &bytes)?;
        // Build the copy-on-write heap image now rather than lazily on the
        // first instantiation, moving that one-time cost off the first call.
        component.initialize_copy_on_write_image()?;
        Ok(component)
    })
    .await
    .context("guest load task panicked")?
}

// Deserialize a pre-compiled component, or compile raw wasm. The base build
// has no compiler and refuses raw wasm; the `jit` feature adds the compile
// path ahead of that refusal.
fn load(engine: &Engine, bytes: &[u8]) -> Result<Component> {
    let precompiled = Engine::detect_precompiled(bytes);
    #[cfg(feature = "jit")]
    if precompiled.is_none() {
        return Component::new(engine, bytes)
            .map_err(anyhow::Error::from)
            .context("compiling component");
    }
    match precompiled {
        Some(Precompiled::Component) => {
            // SAFETY: deployment inputs are trusted operator inputs
            // (docs/security-model.md): these bytes are the artifact the
            // deployment declared, and a pre-compiled one is `omnia compile`
            // output by that declaration — the contract
            // `Component::deserialize` requires.
            unsafe { Component::deserialize(engine, bytes) }
                .map_err(anyhow::Error::from)
                .with_context(|| format!("deserializing pre-compiled component: {SETTINGS_HINT}"))
        }
        Some(Precompiled::Module) => {
            bail!("the artifact is a pre-compiled core module, not a component")
        }
        None => bail!(
            "compiling raw wasm requires the `jit` feature; pre-compile the component with \
             `omnia compile` instead"
        ),
    }
}
