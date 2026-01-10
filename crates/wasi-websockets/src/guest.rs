//! # WASI WebSockets WIT implementation

#![allow(clippy::same_length_and_capacity)]

// Bindings for the `wasi:websockets` world.
// See (<https://github.com/augentic/wasi-websockets/>)
wit_bindgen::generate!({
    world: "websockets",
    path: "wit",
    generate_all,
    pub_export_macro: true
});

pub use self::wasi::websockets::*;
