//! # WASI Model Guest
//!
//! Guest-side bindings for the `omnia:model` worlds. A guest imports
//! `omnia:model/completion` and calls `complete`.

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

/// Bindings for the `run` reactor world (`import completion`, `export run`).
pub mod run {
    mod generated {
        #![allow(missing_docs)]
        wit_bindgen::generate!({
            world: "run",
            path: "wit",
            pub_export_macro: true,
            with: {
                "wasi:filesystem/types@0.3.0": wasip3::filesystem::types,
                "wasi:clocks/system-clock@0.3.0": wasip3::clocks::system_clock,
                "wasi:clocks/types@0.3.0": wasip3::clocks::types,
            },
        });
    }

    pub use generated::*;
    pub use generated::omnia::model::completion;
}

/// Bindings for the `shelf` world (`export references`).
pub mod shelf {
    mod generated {
        #![allow(missing_docs)]
        wit_bindgen::generate!({
            world: "shelf",
            path: "wit",
            pub_export_macro: true,
        });
    }

    pub use generated::*;
}

pub use self::model::omnia::model::*;
