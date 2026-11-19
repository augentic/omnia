//! # WASI WebSocket Guest

// Bindings for `omnia:websocket`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "websocket",
        path: "wit",
        with: {
            "omnia:types/types@0.1.0": omnia_wasi_types::omnia::types,
        },
        additional_derives: [Clone],
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "omnia_wasi_websocket",
    });
}

/// `omnia:websocket`, the interfaces behind the SDK's `WebSockets`, and the
/// handler a guest exports to receive messages.
pub mod omnia {
    pub use crate::guest::generated::exports::omnia::websocket::*;
    pub use crate::guest::generated::omnia::websocket::*;
}

// `export!` reaches the handler's glue through the crate root
pub use self::generated::export;
#[doc(hidden)]
pub use self::generated::exports;
