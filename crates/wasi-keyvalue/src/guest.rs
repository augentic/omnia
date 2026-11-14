//! # WASI Key-Value Guest

// Bindings for `omnia:keyvalue` and the `wasi:keyvalue` draft.
// See (<https://github.com/WebAssembly/wasi-keyvalue/>)
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
    world: "imports",
    path: "wit",
    generate_all,
    });
}

/// `omnia:keyvalue`, the interface the SDK's `StateStore` calls.
pub mod omnia {
    pub use crate::guest::generated::omnia::keyvalue::store;
}

pub use self::generated::wasi::keyvalue::*;
