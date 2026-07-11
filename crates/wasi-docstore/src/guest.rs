//! # WASI `DocStore` Guest

// Bindings for the `wasi:docstore` world.
mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

mod convert;
pub mod store;
