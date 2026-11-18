//! # WASI WebSocket Guest

// Bindings for `omnia:websocket`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "websocket",
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
