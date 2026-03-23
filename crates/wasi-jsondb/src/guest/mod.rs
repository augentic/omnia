//! Guest-side `wasi:jsondb` bindings and helpers.

pub(crate) mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

mod convert;
pub mod store;
