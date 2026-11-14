//! # WASI SQL Guest

// Bindings for `omnia:sql` and the `wasi:sql` draft.
// See (<https://github.com/augentic/wasi-sql/>)
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:sql`, the interface the SDK's `SqlDb` calls.
pub mod omnia {
    pub use crate::guest::generated::omnia::sql::database;
}

pub use self::generated::wasi::sql::*;
pub use crate::types::{DataType, Field, Row};
