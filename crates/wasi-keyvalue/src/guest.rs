//! # WASI Key-Value Guest

// Bindings for the `wasi:keyvalue` and `omnia:keyvalue` worlds.
mod generated {
    #![expect(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::omnia::keyvalue::ttl;
pub use self::generated::wasi::keyvalue::*;
