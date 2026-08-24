//! Locating pre-built guest components in integration tests.
//!
//! [`find_guest`] is locate-only: tests never invoke Cargo. Guests are built
//! (and serialized) up front by `cargo make test-guests`; a missing
//! artifact fails fast with that instruction, locally and in CI alike.

use std::env;
use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};
use omnia::GuestArtifact;

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

    panic!("guest `{file}` not built; run:\n  cargo make test-guests");
}

/// Read the serialized `.bin` bytes for `file`, failing fast (rather than
/// substituting raw wasm) when the `.bin` is missing, so the pre-compiled
/// path is genuinely exercised.
///
/// # Errors
///
/// Returns an error when the `.bin` is missing or unreadable.
pub fn precompiled_bytes(file: &str) -> Result<Vec<u8>> {
    let path = find_guest(file);
    ensure!(
        path.extension().is_some_and(|ext| ext == "bin"),
        "{} has no serialized .bin sibling; run `cargo make test-guests`",
        path.display()
    );
    std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))
}

/// Read the serialized `.bin` for `file` and wrap it as a pre-compiled
/// registration artifact.
///
/// # Errors
///
/// Returns an error when the `.bin` is missing or unreadable.
// The one place the test tiers attest artifact trust; every suite shares it
// instead of repeating the `unsafe` block.
#[allow(unsafe_code)]
pub fn precompiled_artifact(file: &str) -> Result<GuestArtifact> {
    let bytes = precompiled_bytes(file)?;
    // SAFETY: the artifact was built and serialized by this workspace's own
    // `cargo make test-guests` pipeline (omnia's compile path).
    Ok(unsafe { GuestArtifact::precompiled(bytes) })
}

/// Read the raw `.wasm` sibling bytes for `file` (never the serialized
/// `.bin`), so the safe raw-bytes path is genuinely exercised.
///
/// # Errors
///
/// Returns an error when the `.wasm` is missing or unreadable.
pub fn wasm_bytes(file: &str) -> Result<Vec<u8>> {
    let path = find_guest(file).with_extension("wasm");
    std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))
}

/// The raw-wasm dual of [`precompiled_artifact`], exercising the safe JIT
/// constructor.
///
/// # Errors
///
/// Returns an error when the `.wasm` is missing or unreadable.
pub fn raw_wasm(file: &str) -> Result<GuestArtifact> {
    Ok(GuestArtifact::wasm(wasm_bytes(file)?))
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
