//! # WASI Identity WIT implementation

// Bindings for the `omnia:identity` world.
// See (<https://github.com/augentic/wasi-vault/>)
mod generated {
    #![expect(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::omnia::identity::*;
