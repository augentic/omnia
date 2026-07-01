//! # WASI Model Guest
//!
//! Guest-side bindings for the `omnia:model` world. A guest imports
//! `omnia:model/completion` and calls `complete`.

mod generated {
    #![allow(missing_docs)]
    wit_bindgen::generate!({
        world: "model",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::omnia::model::*;
