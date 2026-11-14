//! [`WasiProvider`], its capability impls, and the guest's WASI exports.
//!
//! On `wasm32`, `WasiProvider` serves each capability through the matching
//! `omnia-wasi-*` guest bindings. A capability that addresses named bindings
//! returns a `Handle` from its `named`:
//!
//! - `identity::Handle`
//! - `keyvalue::Handle`
//! - `sql::Handle`
//! - `vault::Handle`
//!
//! Each transport's `export!` binds its WASI export to the fn here that a
//! hand-written export calls too:
//!
//! - `http::serve` for `wasi:http`, behind the `http` feature
//! - `messaging::handle` for `wasi:messaging`
//! - `command::execute` for `wasi:cli/run`

mod blobstore;
pub mod command;
mod config;
mod docstore;
pub mod http;
pub mod identity;
pub mod keyvalue;
mod lend;
pub mod messaging;
mod model;
mod plugins;
pub mod sql;
pub mod vault;
mod vcs;
mod websockets;

/// Zero-sized WASI provider implementing every capability trait on `wasm32`.
///
/// Delegates to the `omnia-wasi-*` guest bindings on `wasm32-wasip2`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WasiProvider;
