//! Locating pre-built guest components in integration tests.
//!
//! [`find_guest`] encodes the "fail in CI, skip locally" policy so a missing
//! guest never lets CI pass vacuously.

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

static GUESTS: OnceLock<()> = OnceLock::new();

/// Locate a built guest, or signal that the caller should skip.
///
/// Builds example guests on first use, then returns the path when present. When
/// still absent it panics under CI (`CI` set) so the pipeline never passes
/// vacuously, and otherwise returns `None` so local runs skip gracefully.
///
/// # Panics
///
/// Panics under CI (`CI` set) or `build_guests()` fails so the pipeline never
/// passes vacuously.
#[must_use]
pub fn find_guest(file: &str) -> Option<PathBuf> {
    if guest_wasm(file).is_none() {
        build_guests();
    }

    if let Some(path) = guest_wasm(file) {
        return Some(path);
    }

    assert!(
        env::var_os("CI").is_none(),
        "guest `{file}` not built under CI; run:\n  cargo build -p examples --examples --target wasm32-wasip2"
    );
    eprintln!("skipping: guest `{file}` not built.");

    None
}

// Locate a built guest component by file name, preferring the debug profile.
fn guest_wasm(file: &str) -> Option<PathBuf> {
    let target = get_target_dir();

    // find file in debug or release profiles
    ["debug", "release"]
        .into_iter()
        .map(|profile| target.join("wasm32-wasip2").join(profile).join("examples").join(file))
        .find(|path| path.exists())
}

// Build wasm32-wasip2 example guests.
fn build_guests() {
    GUESTS.get_or_init(|| {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|crates| crates.parent())
            .expect("testkit manifest dir is <workspace>/crates/testkit")
            .to_path_buf();
        let target = get_target_dir();

        let status = Command::new("cargo")
            .env("CARGO_TARGET_DIR", &target)
            .args(["build", "-p", "examples", "--examples", "--target", "wasm32-wasip2"])
            .current_dir(&workspace)
            .status()
            .expect("spawning guest build");

        assert!(status.success(), "guest build failed with status {status}");
    });
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
