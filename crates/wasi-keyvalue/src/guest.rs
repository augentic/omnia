//! # WASI Key-Value Guest

#![allow(clippy::same_length_and_capacity)]

// Bindings for the `wasi:keyvalue` world.
// See (<https://github.com/WebAssembly/wasi-keyvalue/>)
mod generated {
    #![allow(missing_docs)]
    wit_bindgen::generate!({
    world: "keyvalue",
    path: "wit",
    generate_all,
    });
}

pub mod cache;

pub use self::generated::wasi::keyvalue::*;
