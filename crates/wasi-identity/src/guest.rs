//! # WASI Identity WIT implementation

// Bindings for the `wasi:vault` world.
// See (<https://github.com/augentic/wasi-vault/>)
mod generated {
    #![allow(missing_docs)]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::omnia::identity::*;
