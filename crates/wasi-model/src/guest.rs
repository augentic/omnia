//! # WASI Model Guest

// Bindings for `omnia:model`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        with: {
            "wasi:filesystem/types@0.3.2": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.2": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.2": wasip3::clocks::types,
        },
    });
}

/// `omnia:model`, the interfaces behind the SDK's `Model`.
pub mod omnia {
    pub use crate::guest::generated::omnia::model::*;
}

pub use self::generated::{wit_future, wit_stream};
