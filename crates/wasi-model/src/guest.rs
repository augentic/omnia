//! # WASI Model Guest
//!
//! Guest-side bindings for the `omnia:model` world. A guest imports
//! `omnia:model/completion` and calls `create`.

mod model {
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

pub use self::model::omnia::model::*;
