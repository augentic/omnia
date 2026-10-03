//! # WASI Key-Value Guest

// Bindings for the `wasi:keyvalue` world.
// See (<https://github.com/WebAssembly/wasi-keyvalue/>)
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
    world: "imports",
    path: "wit",
    generate_all,
    });
}

pub mod cache;

pub use self::generated::wasi::keyvalue::*;
