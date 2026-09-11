//! Wasmtime's `create_rust_wasm` + `create_rust_test` pattern over the
//! examples: build the guests for `wasm32-wasip2`, then `cargo run --example`
//! each run-to-completion host from the workspace root and require exit
//! status 0. Examples assume the default `target/` directory — a redirected
//! `CARGO_TARGET_DIR` is unsupported for this tier, unlike `test-programs` —
//! and nothing else is asserted: behaviour lives in the `crates/wasi-*/tests`
//! suites, and the server examples are intentionally build-only.

use std::path::Path;
use std::process::Command;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root")
}

fn cargo(args: &[&str]) {
    let status = Command::new(env!("CARGO"))
        .arg("--locked")
        .args(args)
        .current_dir(root())
        .status()
        .expect("spawn cargo");
    assert!(status.success(), "cargo {} failed", args.join(" "));
}

/// Idempotent: nextest runs every test in its own process, so each one builds
/// first; concurrent invocations serialise on cargo's lock and the rest are
/// no-ops.
fn build_guests() {
    cargo(&["build", "-p", "examples", "--examples", "--target", "wasm32-wasip2"]);
}

fn run(example: &str, args: &[&str]) {
    build_guests();
    cargo(&[&["run", "-p", "examples", "--example", example, "--"], args].concat());
}

#[test]
fn build() {
    cargo(&["build", "-p", "examples", "--examples"]);
}

#[test]
fn model() {
    run("model", &[]);
}

#[test]
fn cli() {
    run("cli", &["run", "target/wasm32-wasip2/debug/examples/cli_wasm.wasm", "--", "greet", "Ada"]);
}

#[test]
fn cli_static() {
    run("cli-static", &["greet", "Ada"]);
}

#[test]
fn guest_link_dynamic() {
    run("guest-link-dynamic", &[]);
}

#[test]
fn guest_link_register() {
    run("guest-link-register", &[]);
}
