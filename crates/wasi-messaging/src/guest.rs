//! # WASI Messaging Guest

// Bindings for `omnia:messaging`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "messaging",
        path: "wit",
        with: {
            "omnia:types/types@0.1.0": omnia_wasi_types::omnia::types,
        },
        additional_derives: [Clone],
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "omnia_wasi_messaging",
    });
}

/// `omnia:messaging`, the interfaces behind the SDK's `Publisher`, and the
/// handler a subscribing guest exports.
pub mod omnia {
    pub use crate::guest::generated::exports::omnia::messaging::*;
    pub use crate::guest::generated::omnia::messaging::*;
}

// `export!` reaches the handler's glue through the crate root
pub use self::generated::export;
#[doc(hidden)]
pub use self::generated::exports;
