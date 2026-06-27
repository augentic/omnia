//! # CLI Command Wasm Guest
//!
//! A `wasi:cli/command` reactor: a `cdylib` exporting `wasi:cli/run@0.3.0` via
//! [`wasip3::cli::command::export!`], in the same shape as every other Omnia
//! example (a `cdylib` whose exported handler the host drives). The host drives
//! `run` exactly **once** and exits with its status.
//!
//! It dispatches a small set of subcommands on the argv the host injects (read
//! through the p2 `std` bridge, which Omnia links alongside p3), writes to
//! stdout/stderr, and returns:
//!
//! - `greet [name]` — prints `Hello, <name>!` (default `world`).
//! - `add [n...]`   — prints the sum of its integer arguments.
//! - `env`          — prints the inherited environment, one `key=value` per line.
//!
//! An unknown subcommand exits with a specific code via
//! [`wasip3::cli::exit::exit_with_code`] (which the host observes as the exit
//! status through `wasmtime`'s `I32Exit`); missing usage returns `Err(())`,
//! which the host maps to `1`. (`run` alone only distinguishes success from
//! failure, so a specific code needs `wasi:cli/exit`.)
//!
//! Because `cargo build`/`cargo test` also compile examples for the host triple
//! — where the wasm-only `wasip3` crate is unavailable — the whole module is
//! guarded with `#[cfg(target_arch = "wasm32")]` (a `cdylib` needs no `main`).

#![cfg(target_arch = "wasm32")]

use wasip3::exports::cli::run::Guest;

struct Cli;
wasip3::cli::command::export!(Cli);

impl Guest for Cli {
    /// The `wasi:cli/run` export: dispatch on argv, then signal success or a
    /// process exit code.
    async fn run() -> Result<(), ()> {
        let args: Vec<String> = std::env::args().collect();

        // args[0] is the program name (supplied by the host); args[1] is the
        // subcommand.
        match args.get(1).map(String::as_str) {
            Some("greet") => {
                let who = args.get(2).map(String::as_str).unwrap_or("world");
                println!("Hello, {who}!");
            }
            Some("add") => {
                let sum: i64 = args[2..].iter().filter_map(|a| a.parse::<i64>().ok()).sum();
                println!("{sum}");
            }
            Some("env") => {
                for (key, value) in std::env::vars() {
                    println!("{key}={value}");
                }
            }
            Some(other) => {
                eprintln!("unknown command: {other}");
                // A specific nonzero code: `wasi:cli/exit`'s exit-with-code
                // surfaces host-side as wasmtime's `I32Exit`, which `WasiCli`
                // maps to the process exit status — fidelity beyond the plain
                // success/failure `run` returns.
                wasip3::cli::exit::exit_with_code(2);
            }
            None => {
                eprintln!("usage: <greet|add|env> [args...]");
                // A plain failure: `run` returning `Err(())` maps to exit 1.
                return Err(());
            }
        }

        Ok(())
    }
}
