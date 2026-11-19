#![doc = include_str!("../README.md")]
//!
//! # The rungs
//!
//! - **Component rung** ([`host`]) — the compiled `wasm32-wasip3` component
//!   driven through omnia's own runtime over [`Backends`](host::Backends):
//!   the in-memory defaults, any of them swapped for a scripted or recording
//!   backend. [`host::send`] and [`host::deliver`] drive one request or one
//!   message through it in process.
//! - **Contract rows** ([`contract`]) — the invariants every backend of an
//!   `omnia:*` interface meets, run by a guest component over the
//!   `omnia_sdk` bindings.
//! - **Fixture rung** ([`build`]) — the nested cargo build a consumer's
//!   `build.rs` runs to produce those components and the `gen.rs` naming them.
//!
//! [`Script`] underlies the scripted model: a shared FIFO of turns that
//! records the requests consuming them and fails the test when turns are
//! left over.
#[cfg(feature = "contract")]
pub mod contract;

cfg_select! {
    not(target_arch = "wasm32") => {
        mod script;

        #[cfg(feature = "build")]
        pub mod build;
        #[cfg(feature = "host")]
        pub mod host;

        pub use script::{Exchange, Script, Seen, SeenFormat, SeenWorker};
    }
    _ => {}
}
