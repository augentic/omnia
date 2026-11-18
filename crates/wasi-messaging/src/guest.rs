//! # WASI Messaging Guest

// Bindings for `omnia:messaging`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "messaging",
        path: "wit",
        additional_derives: [Clone],
        generate_all,
        pub_export_macro: true,
    });
}

pub use self::generated::exports::omnia::messaging::*;
pub use self::generated::omnia::messaging::*;
pub use self::generated::*;
