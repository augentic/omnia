//! # WASI Model Guest

// Bindings for the `wasi:model` world.
// See (<https://github.com/WebAssembly/wasi-model/>)
mod generated {
    #![allow(missing_docs)]
    wit_bindgen::generate!({
        world: "model",
        path: "wit",
        with: {
            "wasi:filesystem/types@0.3.0": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.0": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.0": wasip3::clocks::types,
        },
    });
}

pub mod prompt;

pub use self::generated::omnia::model::*;
pub use self::generated::{wit_future, wit_stream};
