//! # WASI Config Guest

// Bindings for `omnia:config`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        with: {
            "omnia:types/types@0.1.0": omnia_wasi_types::omnia::types,
        },
        generate_all,
    });
}

/// `omnia:config`, the interfaces behind the SDK's `Config`.
pub mod omnia {
    pub use crate::guest::generated::omnia::config::*;
}
