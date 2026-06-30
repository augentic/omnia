//! # Compiler

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use wasmtime::component::Component;
use wasmtime::{Config, Engine};

use crate::RuntimeOptions;

/// Compile `wasm32-wasip2` component.
///
/// For example, to compile the `http` component, run:
///
/// ```bash
/// cargo build --package otel --target wasm32-wasip2 --release
/// ```
///
/// # Errors
///
/// Returns an error if the wasm component cannot be loaded from the specified
/// path, cannot be compiled, or cannot be serialized to the specified output
/// directory.
pub fn compile(wasm: &Path, output: Option<PathBuf>) -> Result<()> {
    let Some(file_name) = wasm.file_name() else {
        return Err(anyhow!("invalid file name"));
    };

    // compile component (compile-time config must match the loader in `create`)
    let options = RuntimeOptions::load()?;
    let wt_config = &Config::from(&options);

    let engine = Engine::new(wt_config)?;
    let component = Component::from_file(&engine, wasm)?;
    let serialized = component.serialize()?;

    // output to file or stdout
    if let Some(mut out_path) = output {
        // output to file
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
        // output to stdout
        let mut stdout = io::stdout().lock();
        stdout.write_all(&serialized)?;
    }

    Ok(())
}
