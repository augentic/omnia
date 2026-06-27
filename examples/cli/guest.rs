//! # CLI Command Wasm Guest
//!
//! A `wasi:cli/command` component: a plain Rust binary that the `wasm32-wasip2`
//! target maps onto the `wasi:cli/run` export (no `wit-bindgen`, no
//! `crate-type`). It dispatches a small set of subcommands on the argv the host
//! injects via `wasi:cli/environment`, writes to stdout/stderr, and exits.
//!
//! - `greet [name]` — prints `Hello, <name>!` (default `world`).
//! - `add [n...]`   — prints the sum of its integer arguments.
//! - `env`          — prints the inherited environment, one `key=value` per line.
//!
//! A nonzero [`std::process::exit`] (or a panic) surfaces as `Err(())` from
//! `wasi:cli/run`, which the host maps to a nonzero process exit.
//!
//! Because `cargo build`/`cargo test` also compile examples for the host
//! triple, the real entrypoint is guarded with `#[cfg(target_arch = "wasm32")]`
//! and an empty `main` is supplied for every other target.

#[cfg(target_arch = "wasm32")]
fn main() {
    let args: Vec<String> = std::env::args().collect();

    // args[0] is the program name; args[1] is the subcommand.
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
            std::process::exit(2);
        }
        None => {
            eprintln!("usage: <greet|add|env> [args...]");
            std::process::exit(1);
        }
    }
}

// A binary example needs a `main` when cargo builds it for the host target.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
