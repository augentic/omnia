//! # WASI `DocStore` Guest

// Bindings for `omnia:docstore`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:docstore`, the interfaces behind the SDK's `DocumentDb`.
pub mod omnia {
    pub use crate::guest::generated::omnia::docstore::*;
}
