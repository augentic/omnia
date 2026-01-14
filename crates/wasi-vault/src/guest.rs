//! # WASI Vault WIT implementation

#![allow(clippy::same_length_and_capacity)]

// Bindings for the `wasi:vault` world.
// See (<https://github.com/augentic/wasi-vault/>)
mod generated {
    #![allow(missing_docs)]
    wit_bindgen::generate!({
        world: "vault",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::wasi::vault::*;
