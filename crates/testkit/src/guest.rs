//! Locating pre-built guest components in integration tests.
//!
//! Guests compile to `target/wasm32-wasip2/<profile>/examples/<name>.wasm`; the
//! `cargo make build-guests` task builds them before tests run. [`find_guest`]
//! encodes the "fail in CI, skip locally" policy so a missing guest never lets
//! CI pass vacuously.

use std::path::PathBuf;

/// Locate a built guest, or signal that the caller should skip.
///
/// Returns the guest path when present. When absent it panics under CI (`CI`
/// set) so the pipeline never passes vacuously, and otherwise returns `None`
/// with a build hint so local runs skip gracefully.
///
/// # Panics
///
/// Panics when the guest is missing and `CI` is set.
#[must_use]
pub fn find_guest(file: &str, build_hint: &str) -> Option<PathBuf> {
    if let Some(path) = guest_wasm(file) {
        return Some(path);
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "guest `{file}` not built under CI; run:\n  {build_hint}"
    );
    eprintln!("skipping: guest `{file}` not built. Run:\n  {build_hint}");
    None
}

/// Locate a built guest component by file name, preferring the debug profile.
fn guest_wasm(file: &str) -> Option<PathBuf> {
    let target = target_dir();
    ["debug", "release"]
        .into_iter()
        .map(|profile| target.join("wasm32-wasip2").join(profile).join("examples").join(file))
        .find(|path| path.exists())
}

/// The workspace `target/` directory, derived from the test executable at
/// `<target>/<profile>/deps/<exe>`.
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test executable has a path");
    exe.ancestors().nth(3).expect("test exe sits at <target>/<profile>/deps/<exe>").to_path_buf()
}
