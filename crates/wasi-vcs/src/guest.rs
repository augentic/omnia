//! # WASI VCS Guest

// Bindings for `omnia:vcs`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        with: {
            "omnia:types/types@0.1.0": omnia_wasi_types::omnia::types,
            "wasi:filesystem/types@0.3.2": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.2": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.2": wasip3::clocks::types,
        },
    });
}

/// `omnia:vcs`, the interfaces behind the SDK's `Vcs`.
pub mod omnia {
    pub use crate::guest::generated::omnia::vcs::*;
}
