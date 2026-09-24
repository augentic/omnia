//! # Compiler
//!
//! Ahead-of-time compilation of a component into the artifact the runtime
//! deserializes instead of compiling at startup.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};
use omnia_core::CompileOptions;
use wasmtime::{Config, Engine};

/// Compile the `wasm32-wasip2` component at `wasm` ahead of time.
///
/// The artifact is written to `output` — a file path (written exactly there,
/// parent directories created) or an existing directory (the artifact lands
/// inside it as `<input stem>.bin`) — or to stdout when `output` is `None`.
/// `target` is the triple the artifact runs on; `None` compiles for the host.
/// `options` are the compile-affecting settings, and the runtime that loads
/// the artifact must run under the same ones: [`CompileOptions::default`]
/// matches a runtime whose environment sets none of them, and
/// `RuntimeOptions::load_env()?.compile_options()` matches the environment
/// the compile runs in.
///
/// Nothing runtime-only — the pooling allocator, backtrace limits — reaches
/// the compiler's engine, so a foreign `target` compiles.
///
/// # Errors
///
/// Returns an error if `wasm` has no file name or cannot be read, `target`
/// is not a triple the compiler supports, the component does not compile, or
/// the artifact cannot be written.
pub fn compile(
    wasm: &Path, output: Option<PathBuf>, target: Option<&str>, options: &CompileOptions,
) -> Result<()> {
    let Some(file_name) = wasm.file_name() else {
        return Err(anyhow!("invalid file name"));
    };

    // configure the compiler's engine
    let mut config = Config::new();
    options.configure(&mut config);
    if let Some(triple) = target {
        config
            .target(triple)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("targeting `{triple}`"))?;
    }
    let engine = Engine::new(&config)?;

    // compile
    let bytes = fs::read(wasm).with_context(|| format!("reading {}", wasm.display()))?;
    let serialized = engine
        .precompile_component(&bytes)
        .map_err(anyhow::Error::from)
        .context("compiling component")?;

    // write the artifact
    if let Some(mut out_path) = output {
        if out_path.is_dir() {
            out_path.push(file_name);
            out_path.set_extension("bin");
        }
        if let Some(dir) = out_path.parent()
            && !dir.exists()
        {
            fs::create_dir_all(dir)?;
        }
        File::create(&out_path)?.write_all(&serialized)?;
    } else {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&serialized)?;
    }

    Ok(())
}
