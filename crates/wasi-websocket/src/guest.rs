//! # WASI WebSocket WIT implementation

// Bindings for the `wasi:websocket` world.
// See (<https://github.com/augentic/wasi-websocket/>)
mod generated {
    #![allow(missing_docs)]
    #![allow(clippy::same_length_and_capacity)]

    wit_bindgen::generate!({
        world: "websocket",
        path: "wit",
        additional_derives: [Clone],
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "qwasr_wasi_websocket",
    });
}

pub use self::generated::exports::wasi::websocket::*;
pub use self::generated::wasi::websocket::*;
pub use self::generated::*;
