//! # WASI Model Guest
//!
//! Guest-side bindings for the `augentic:model` world. A guest imports
//! `augentic:model/completion` and calls `complete`.

mod generated {
    #![allow(missing_docs)]
    wit_bindgen::generate!({
        world: "model",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::augentic::model::*;
