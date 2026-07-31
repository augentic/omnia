//! # WASI SQL WIT implementation

// Bindings for the `wasi:sql` world.
// See (<https://github.com/augentic/wasi-sql/>)
mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::wasi::sql::*;
pub use crate::types::{DataType, Field, Row};
