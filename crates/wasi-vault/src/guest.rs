//! # WASI Vault Guest

// Bindings for `omnia:vault`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:vault`, the interfaces behind the SDK's `SecretStore`.
pub mod omnia {
    pub use crate::guest::generated::omnia::vault::*;
}
