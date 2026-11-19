//! Components compiled ahead of time once, for every boot after the first.

use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::Path;

use anyhow::{Context as _, Result};
use omnia::wasmtime::{Config, Engine};
use omnia::{Digest, RuntimeOptions, SourceSpec};

/// The component at `wasm`, compiled ahead of time once and shared by every
/// boot after.
///
/// Booting a raw component compiles it, and that compile dominates a suite
/// that boots a deployment per test. The artifact is cached in a
/// `precompiled` directory beside the component, named for the component's
/// content and the engine the runtime builds from the environment. Every
/// later call, in any test process, reads it back instead. Pass the result
/// to [`Deployment::guest`](super::Deployment::guest).
///
/// # Errors
///
/// Returns an error if the runtime's options cannot be read from the
/// environment, the component cannot be read or compiled, or the artifact
/// cannot be written or read back.
pub fn precompiled(wasm: impl AsRef<Path>) -> Result<SourceSpec> {
    let wasm = wasm.as_ref();
    let options = RuntimeOptions::load_env()?.compile_options();

    // name the artifact for what decides its bytes
    let component = fs::read(wasm).with_context(|| format!("reading {}", wasm.display()))?;
    let mut config = Config::new();
    options.configure(&mut config);
    let engine = Engine::new(&config)?;
    let mut hasher = DefaultHasher::new();
    Digest::of(&component).hash(&mut hasher);
    engine.precompile_compatibility_hash().hash(&mut hasher);
    let stem = wasm.file_stem().map_or_default(|stem| stem.to_string_lossy().into_owned());
    let dir = wasm.with_file_name("precompiled");
    let artifact = dir.join(format!("{stem}-{:016x}.bin", hasher.finish()));

    // compile under a lock, so concurrent test processes compile once
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let lock = File::create(artifact.with_extension("lock"))?;
    lock.lock().context("locking the precompiled artifact")?;
    if !artifact.exists() {
        let staged = artifact.with_extension("partial");
        omnia::compile::compile(wasm, Some(staged.clone()), None, &options)?;
        fs::rename(&staged, &artifact)?;
    }
    let bytes = fs::read(&artifact).with_context(|| format!("reading {}", artifact.display()))?;
    drop(lock);

    Ok(bytes.into())
}
