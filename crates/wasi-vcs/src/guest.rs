//! # WASI VCS Guest

// Bindings for the `omnia:vcs` world.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        with: {
            "wasi:filesystem/types@0.3.0": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.0": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.0": wasip3::clocks::types,
        },
    });
}

pub use self::generated::omnia::vcs::*;
