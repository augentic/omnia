//! # WASI Types Guest

use std::fmt;

// Bindings for `omnia:types`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
        generate_unused_types: true,
    });
}

/// `omnia:types`, the types every `omnia-wasi-*` guest binding shares.
///
/// Each binding maps `omnia:types/types` here, so a guest sees one
/// [`Error`](types::Error) from every capability.
pub mod omnia {
    pub use crate::guest::generated::omnia::types::*;
}

// wit-bindgen implements these only for an `error` a function in the same
// world returns, and this world has no function
impl fmt::Display for omnia::types::Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for omnia::types::Error {}
