//! # WASI SQL WIT implementation

#![allow(clippy::same_length_and_capacity)]

// Bindings for the `wasi:sql` world.
// See (<https://github.com/augentic/wasi-sql/>)
mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "sql",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::wasi::sql::types::{DataType, Field, Row};
pub use self::generated::wasi::sql::*;
