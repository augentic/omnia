//! # WASI `DocStore` Guest

// Bindings for the `omnia:docstore` world.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::omnia::docstore::*;
