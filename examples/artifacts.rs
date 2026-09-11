//! Where the example guests land once built for `wasm32-wasip2`: cargo's
//! target directory, honouring `CARGO_TARGET_DIR` when set.

use std::path::{Path, PathBuf};

/// Absolute path of a built example guest, e.g. `artifact("cli_wasm.wasm")`.
pub fn artifact(guest: &str) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"), PathBuf::from)
        .join("wasm32-wasip2/debug/examples")
        .join(guest)
}
