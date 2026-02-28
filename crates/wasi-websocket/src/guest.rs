//! # WASI WebSocket WIT implementation

// Bindings for the `wasi:websocket` world.
// See (<https://github.com/augentic/wasi-websocket/>)
mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "duplex",
        path: "wit",
        additional_derives: [Clone],
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "omnia_wasi_websocket",
    });
}

pub use self::generated::exports::omnia::websocket::*;
pub use self::generated::omnia::websocket::*;
pub use self::generated::*;
