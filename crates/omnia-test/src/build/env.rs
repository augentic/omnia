//! The build-script environment: cargo's variables, the nested command, and
//! the sanitising that keeps outer host flags out of the wasm32 build.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const WASM_TARGET: &str = "wasm32-wasip2";

/// The nested build's target directory, a sibling of the outer profile
/// directory.
const NESTED_TARGET: &str = "wasm32-fixtures";

/// A cargo-provided variable, or a panic naming it: the pipeline only runs
/// under `cargo` as a build script.
pub fn var(name: &str) -> OsString {
    env::var_os(name).unwrap_or_else(|| panic!("`{name}` is set by cargo for build scripts"))
}

pub fn path_var(name: &str) -> PathBuf {
    PathBuf::from(var(name))
}

/// Whether the outer build targets `wasm32`, where nesting a fixture build
/// would recurse.
pub fn outer_target_is_wasm32() -> bool {
    env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32")
}

/// Whether the outer `RUSTFLAGS` deny warnings, the one flag propagated into
/// the nested build.
pub fn denies_warnings(rustflags: Option<&str>) -> bool {
    rustflags.is_some_and(|flags| flags.contains("-Dwarnings") || flags.contains("-D warnings"))
}

/// Where the nested build lands, given the consumer's `OUT_DIR`.
///
/// Cargo lays a build script's `OUT_DIR` out as
/// `<target>/<profile>/build/<package>-<hash>/out` (layout v1) or
/// `<target>/<profile>/build/<package>/<hash>/out` (layout v2), and the hash
/// changes with the package version, the toolchain, and the build
/// dependencies — each change would leave the previous fixture tree behind,
/// never reused and never removed. The nested build therefore goes to
/// `<target>/wasm32-fixtures`, one directory shared by every outer
/// configuration, with its own lock so it never waits on the outer build.
/// An `OUT_DIR` of another shape falls back to `<out_dir>/fixtures`.
pub fn nested_dir(out_dir: &Path) -> PathBuf {
    // v1: `build` is two levels above `out`; v2 inserts `<package>/<hash>`.
    [2usize, 3]
        .into_iter()
        .find_map(|n| {
            out_dir
                .ancestors()
                .nth(n)
                .filter(|dir| dir.file_name().is_some_and(|name| name == "build"))
        })
        .and_then(|build| build.parent()?.parent())
        .map_or_else(|| out_dir.join("fixtures"), |target| target.join(NESTED_TARGET))
}

/// The nested `cargo build` for the fixture components, sanitised and pointed
/// at its own target directory.
pub fn nested_build(root: &Path, target_dir: &Path) -> Command {
    // Reuse the outer cargo to stay on its toolchain; read before sanitising.
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let deny_warnings = denies_warnings(env::var("RUSTFLAGS").ok().as_deref());

    let mut command = Command::new(cargo);
    command.current_dir(root).args(["build", "--locked", "--target", WASM_TARGET]);
    sanitise(&mut command, env::vars_os().filter_map(|(key, _)| key.into_string().ok()));
    command.env("CARGO_TARGET_DIR", target_dir);
    // Wasmtime ignores guest DWARF unless asked for it; it would only inflate
    // the fixture tree and the components the suites load.
    command.env("CARGO_PROFILE_DEV_DEBUG", "0");
    if deny_warnings {
        command.env("RUSTFLAGS", "-Dwarnings");
    }
    command
}

/// Strips the outer build's `CARGO_*` and `RUST*` variables from `command`
/// so host flags do not leak into the wasm32 build, keeping only the settings
/// cargo itself needs (`CARGO_HOME`, offline mode). Returns what was removed.
fn sanitise(command: &mut Command, keys: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut removed: Vec<String> = keys.into_iter().filter(|key| should_strip(key)).collect();
    // The outer command may be `cargo clippy`, whose workspace wrapper would
    // run clippy-driver over the guests' wasm32 dep tree; the fixtures are a
    // plain rustc build. The wrappers are removed even when unset so the
    // nested build never inherits them from a later `env`.
    for key in STRIPPED_TOOLCHAIN_VARS {
        if !removed.iter().any(|removed| removed == key) {
            removed.push((*key).to_owned());
        }
    }
    removed.sort();
    for key in &removed {
        command.env_remove(key);
    }
    removed
}

const KEPT_CARGO_VARS: [&str; 2] = ["CARGO_HOME", "CARGO_NET_OFFLINE"];
const STRIPPED_TOOLCHAIN_VARS: [&str; 5] =
    ["RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"];

fn should_strip(key: &str) -> bool {
    (key.starts_with("CARGO_") && !KEPT_CARGO_VARS.contains(&key))
        || STRIPPED_TOOLCHAIN_VARS.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_warnings() {
        assert!(denies_warnings(Some("-Dwarnings")));
        assert!(denies_warnings(Some("-C target-cpu=native -D warnings")));
        assert!(!denies_warnings(Some("-C target-cpu=native")));
        assert!(!denies_warnings(None));
    }

    // The sibling is chosen from cargo's layout alone — v1
    // (`build/<pkg>-<hash>/out`) and v2 (`build/<pkg>/<hash>/out`) — under
    // the default `target/` and a redirected one alike; a layout that is
    // not cargo's keeps the build under `OUT_DIR`.
    #[test]
    fn nested_target() {
        assert_eq!(
            nested_dir(Path::new("/repo/target/debug/build/test-programs-0a1b/out")),
            Path::new("/repo/target/wasm32-fixtures")
        );
        assert_eq!(
            nested_dir(Path::new("/tmp/cache/cargo-target/release/build/probe-ff/out")),
            Path::new("/tmp/cache/cargo-target/wasm32-fixtures")
        );
        assert_eq!(
            nested_dir(Path::new("/repo/target/x86_64-unknown-linux-gnu/debug/build/p-1/out")),
            Path::new("/repo/target/x86_64-unknown-linux-gnu/wasm32-fixtures")
        );
        assert_eq!(
            nested_dir(Path::new("/repo/target/debug/build/test-programs/0a1b/out")),
            Path::new("/repo/target/wasm32-fixtures")
        );
        assert_eq!(
            nested_dir(Path::new("/tmp/cache/cargo-target/release/build/probe/ff/out")),
            Path::new("/tmp/cache/cargo-target/wasm32-fixtures")
        );
        assert_eq!(
            nested_dir(Path::new("/repo/target/x86_64-unknown-linux-gnu/debug/build/p/1/out")),
            Path::new("/repo/target/x86_64-unknown-linux-gnu/wasm32-fixtures")
        );
        assert_eq!(nested_dir(Path::new("/elsewhere/out")), Path::new("/elsewhere/out/fixtures"));
        assert_eq!(
            nested_dir(Path::new("/repo/build/a/b/c/out")),
            Path::new("/repo/build/a/b/c/out/fixtures")
        );
    }

    #[test]
    fn sanitise_env() {
        let outer = [
            "CARGO_HOME",
            "CARGO_NET_OFFLINE",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_MANIFEST_DIR",
            "CARGO_CFG_TARGET_ARCH",
            "RUSTC_WORKSPACE_WRAPPER",
            "PATH",
            "HOME",
        ];
        let removed = sanitise(&mut Command::new("true"), outer.map(str::to_owned));
        assert_eq!(
            removed,
            [
                "CARGO_CFG_TARGET_ARCH",
                "CARGO_ENCODED_RUSTFLAGS",
                "CARGO_MANIFEST_DIR",
                "RUSTC",
                "RUSTC_WORKSPACE_WRAPPER",
                "RUSTC_WRAPPER",
                "RUSTDOCFLAGS",
                "RUSTFLAGS",
            ]
        );
    }
}
