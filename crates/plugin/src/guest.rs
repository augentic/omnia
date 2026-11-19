//! # `omnia:plugins` Guest

// Bindings for `omnia:plugins`.
mod generated {
    #![expect(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:plugins`, the interfaces behind the SDK's `Plugins`.
pub mod omnia {
    pub use crate::guest::generated::omnia::plugins::*;
}
