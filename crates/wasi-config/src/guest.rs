//! # WASI Config WIT implementation

// Bindings for the `wasi:config` world.
// See (<https://github.com/WebAssembly/wasi-config/>)
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::wasi::config::*;
