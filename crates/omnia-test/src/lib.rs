#![doc = include_str!("../README.md")]
//!
//! # The three rungs
//!
//! - **Handler rung** ([`guest`]) — a handler's own logic, compiled natively,
//!   against one double per `omnia_guest` capability; `provider!` assembles a
//!   provider from them and `delegate!` delegates a hand-written one.
//! - **Component rung** ([`host`]) — the compiled `wasm32-wasip2` component
//!   driven through omnia's own runtime over `Backends`, the twelve in-memory
//!   defaults with the model swapped for a `ScriptedModel`.
//! - **Fixture rung** ([`build`]) — the nested cargo build a consumer's
//!   `build.rs` runs to produce those components and the `gen.rs` naming them.
//!
//! [`Script`] underlies both scripted models: a shared FIFO of turns that
//! records the requests consuming them and fails the test when turns are
//! left over.
#![cfg(not(target_arch = "wasm32"))]

mod script;

#[cfg(feature = "build")]
pub mod build;
#[cfg(feature = "guest")]
pub mod guest;
#[cfg(feature = "host")]
pub mod host;

pub use script::{Exchange, Script, Seen, SeenFormat};
