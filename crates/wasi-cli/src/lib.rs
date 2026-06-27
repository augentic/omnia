//! # WASI CLI trigger
//!
//! Host-side, one-shot trigger for `wasi:cli`. [`WasiCli`] drives the
//! `wasi:cli/run` export of the sole command-capable guest exactly once and
//! reports its exit status, then completes — letting [`omnia::serve`] return so
//! the process exits.
//!
//! It is a trigger like HTTP or messaging, not a long-lived server: it shares
//! the same [`omnia::Runtime`], [`omnia::TriggerRouter`], and instance-per-call
//! instantiation path, but invokes a *command* export instead of looping on a
//! transport. The guest is a `wasi:cli/command` reactor (e.g. built with
//! `wasip3::cli::command::export!`), so the host invokes it through the p3
//! `Command` bindings using the same `run_concurrent` convention the HTTP host
//! uses.
//!
//! The exit code is delivered out of band through an `Arc<OnceLock<ExitStatus>>`
//! because [`omnia::Server::run`] / [`omnia::serve`] return `Result<()>` and
//! discard each server's value; the generated `main` reads the cell at the
//! process boundary.

// Host-only: this crate is never part of a wasm32 build (see `Cargo.toml`), but
// gate it defensively to match the `omnia` crate it builds on.
#![cfg(not(target_arch = "wasm32"))]

mod host;

pub use host::WasiCli;
