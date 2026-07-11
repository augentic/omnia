//! Serialize built guest components with Omnia's compile path.
//!
//! `cargo make build-test-guests` invokes this after building the curated seam
//! guests, writing a `.bin` next to each `.wasm` so tests load pre-compiled
//! components (`Component::deserialize_file`) instead of repeating JIT
//! compilation in every test process.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};

fn main() -> Result<()> {
    let wasm_paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(!wasm_paths.is_empty(), "usage: serialize-guests <guest.wasm>...");

    for wasm in wasm_paths {
        ensure!(wasm.exists(), "guest not built: {}", wasm.display());
        let out_dir = wasm.parent().context("guest path has a parent directory")?.to_path_buf();
        omnia::compile(&wasm, Some(out_dir))
            .with_context(|| format!("serializing {}", wasm.display()))?;
        println!("serialized {}", wasm.with_extension("bin").display());
    }

    Ok(())
}
