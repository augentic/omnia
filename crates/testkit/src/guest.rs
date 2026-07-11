//! Locating pre-built guest components in integration tests.
//!
//! [`find_guest`] is locate-only: tests never invoke Cargo. Guests are built
//! (and serialized) up front by `cargo make build-test-guests`; a missing
//! artifact fails fast with that instruction, locally and in CI alike.

use std::env;
use std::path::PathBuf;

/// Locate a pre-built guest component by file name (e.g. `http_wasm.wasm`),
/// preferring a serialized `.bin` (loaded via `Component::deserialize_file`,
/// skipping JIT compilation) over the raw `.wasm`.
///
/// # Panics
///
/// Panics when no artifact exists, so a test run never passes vacuously and
/// never falls back to compiling guests itself.
#[must_use]
pub fn find_guest(file: &str) -> PathBuf {
    let target = get_target_dir();

    // A serialized artifact sits next to the raw wasm with a `.bin` extension
    // (the layout `omnia compile --output <dir>` produces).
    let serialized = PathBuf::from(file).with_extension("bin");
    for profile in ["debug", "release"] {
        let dir = target.join("wasm32-wasip2").join(profile).join("examples");
        let bin = dir.join(&serialized);
        let wasm = dir.join(file);

        // A `.bin` older than its `.wasm` is a stale serialization of a
        // rebuilt guest; using it would silently test old guest code.
        match (mtime(&bin), mtime(&wasm)) {
            (Some(bin_at), Some(wasm_at)) if bin_at >= wasm_at => return bin,
            (_, Some(_)) => return wasm,
            (Some(_), None) => return bin,
            (None, None) => {}
        }
    }

    panic!("guest `{file}` not built; run:\n  cargo make build-test-guests");
}

fn mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    path.metadata().and_then(|m| m.modified()).ok()
}

fn get_target_dir() -> PathBuf {
    if let Some(dir) = env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(dir);
    }
    // Fallback: derive from the test executable's location
    // (<target>/<profile>/deps/<exe>).
    let test_exe = env::current_exe().expect("test executable has a path");
    test_exe
        .ancestors()
        .nth(3)
        .expect("test exe sits at <target>/<profile>/deps/<exe>")
        .to_path_buf()
}
