//! # CLI Command Wasm Guest
//!
//! A `wasi:cli/command` reactor: a `cdylib` exporting `wasi:cli/run@0.3.0` via
//! [`wasip3::cli::command::export!`]. The host drives `run` exactly **once**
//! and exits with its status.
//!
//! The CLI is ordinary [`clap`] (derive API, trimmed features): argv and
//! stdout/stderr arrive through the p2 `std` bridge Omnia links alongside p3,
//! so `--help`, `--version`, and usage errors need no hand-rolling. Exit codes
//! are the one seam nuance: `Args::parse()` would exit through the p2
//! `wasi:cli/exit`, which carries only success/failure and collapses clap's
//! usage-error `2` to `1` — so `run` uses `try_parse()` and forwards
//! [`clap::Error::exit_code`] through the p3
//! [`wasip3::cli::exit::exit_with_code`], which the host observes as
//! wasmtime's `I32Exit`.
//!
//! The module is `#[cfg(target_arch = "wasm32")]`-guarded because examples
//! also compile for the host triple, where `wasip3` is unavailable.

#![cfg(target_arch = "wasm32")]

use clap::{Parser, Subcommand};
use wasip3::exports::cli::run::Guest;

#[derive(Parser)]
#[command(name = "cli", version, about = "Omnia wasi:cli/command example")]
struct Args {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print a greeting
    Greet {
        /// Who to greet
        #[arg(default_value = "world")]
        name: String,
    },
    /// Print the sum of the integer arguments
    Add {
        /// Integers to sum
        numbers: Vec<i64>,
    },
    /// Print the inherited environment, one key=value per line
    Env,
    /// Exit with CODE via wasi:cli/exit, or fail plainly (exit 1) without it
    Fail {
        /// Specific exit code to carry through wasi:cli/exit
        code: Option<u8>,
    },
}

struct Cli;
wasip3::cli::command::export!(Cli);

impl Guest for Cli {
    async fn run() -> Result<(), ()> {
        // Not `parse()`: see the module docs on p2 vs p3 exit fidelity.
        let args = match Args::try_parse() {
            Ok(args) => args,
            Err(error) => {
                let _ = error.print();
                wasip3::cli::exit::exit_with_code(u8::try_from(error.exit_code()).unwrap_or(1));
                unreachable!("exit_with_code does not return");
            }
        };

        match args.command {
            Cmd::Greet { name } => println!("Hello, {name}!"),
            Cmd::Add { numbers } => println!("{}", numbers.iter().sum::<i64>()),
            Cmd::Env => {
                for (key, value) in std::env::vars() {
                    println!("{key}={value}");
                }
            }
            Cmd::Fail { code: Some(code) } => wasip3::cli::exit::exit_with_code(code),
            Cmd::Fail { code: None } => {
                eprintln!("failing plainly");
                return Err(());
            }
        }

        Ok(())
    }
}
