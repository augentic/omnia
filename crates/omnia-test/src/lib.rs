#![doc = include_str!("../README.md")]
//!
//! # The three rungs
//!
//! - **Handler rung** ([`guest`]) — a handler's own logic, compiled natively,
//!   against one double per `omnia_sdk` capability, bundled by
//!   `guest::Provider` or handed to a hand-written provider directly.
//! - **Backend rung** ([`backend`]) — `wasmtime`-free `*Client` doubles
//!   (`ScriptedModel` and `ScriptedVcs`) for driving `omnia_native::Direct`
//!   and the component runtime harness.
//! - **Component rung** ([`host`]) — the compiled `wasm32-wasip2` component
//!   driven through omnia's own runtime over `Backends`, the twelve in-memory
//!   defaults with the model swapped for a `ScriptedModel`.
//! - **Fixture rung** ([`build`]) — the nested cargo build a consumer's
//!   `build.rs` runs to produce those components and the `gen.rs` naming them.
//!
//! [`Script`] underlies the scripted doubles: a shared FIFO of turns that
//! records the requests consuming them and fails the test when turns are
//! left over.

#[cfg(not(target_arch = "wasm32"))]
mod script;

#[cfg(all(not(target_arch = "wasm32"), feature = "backend"))]
pub mod backend;
#[cfg(all(not(target_arch = "wasm32"), feature = "build"))]
pub mod build;
#[cfg(feature = "conformance")]
pub mod conformance;
#[cfg(all(not(target_arch = "wasm32"), feature = "guest"))]
pub mod guest;
#[cfg(all(not(target_arch = "wasm32"), feature = "host"))]
pub mod host;

#[cfg(not(target_arch = "wasm32"))]
pub use script::{Exchange, Script, Seen, SeenFormat, SeenWorker};
