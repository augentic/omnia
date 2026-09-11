//! Wasmtime-style gate over the examples: build every example (guests for
//! `wasm32-wasip2`, hosts natively) with the same cargo commands the READMEs
//! document, then run each run-to-completion host and require exit status 0.
//! Nothing else is asserted — behaviour lives in the `crates/wasi-*/tests`
//! suites — and the server examples are intentionally build-only.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root")
}

fn target() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), PathBuf::from)
}

fn cargo_build(args: &[&str]) {
    let output = Command::new(env!("CARGO"))
        .args(["build", "--locked", "-p", "examples", "--examples"])
        .args(args)
        .current_dir(root())
        .output()
        .expect("spawn cargo");
    assert!(
        output.status.success(),
        "cargo build {} failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Idempotent: nextest runs every test in its own process, so each one builds
/// first; concurrent invocations serialise on cargo's lock and the rest are
/// no-ops.
fn build_examples() {
    cargo_build(&["--target", "wasm32-wasip2"]);
    cargo_build(&[]);
}

fn run(host: &str, args: &[&str]) -> ExitStatus {
    build_examples();
    Command::new(target().join("debug/examples").join(host))
        .args(args)
        .current_dir(root())
        .status()
        .unwrap_or_else(|err| panic!("spawn {host}: {err}"))
}

fn wasm(guest: &str) -> String {
    target().join("wasm32-wasip2/debug/examples").join(guest).display().to_string()
}

#[test]
fn build() {
    build_examples();
}

#[test]
fn model() {
    assert!(run("model", &[]).success());
}

#[test]
fn cli() {
    assert!(run("cli", &["run", &wasm("cli_wasm.wasm"), "--", "greet", "Ada"]).success());
}

#[test]
fn cli_static() {
    assert!(run("cli-static", &["greet", "Ada"]).success());
}

#[test]
fn guest_link_dynamic() {
    assert!(run("guest-link-dynamic", &[]).success());
}

#[test]
fn guest_link_register() {
    assert!(run("guest-link-register", &[]).success());
}
