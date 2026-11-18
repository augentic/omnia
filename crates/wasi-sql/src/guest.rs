//! # WASI SQL Guest

// Bindings for `omnia:sql`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:sql`, the interfaces behind the SDK's `SqlDb`.
pub mod omnia {
    pub use crate::guest::generated::omnia::sql::*;
}
