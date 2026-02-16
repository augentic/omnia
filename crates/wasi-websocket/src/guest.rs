//! # WASI WebSocket WIT implementation

#![allow(clippy::same_length_and_capacity)]

// Bindings for the `wasi:websocket` world.
// See (<https://github.com/augentic/wasi-websocket/>)
mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "websocket",
        path: "wit",
        generate_all,
        pub_export_macro: true
    });
}

pub use self::generated::wasi::websocket::*;
