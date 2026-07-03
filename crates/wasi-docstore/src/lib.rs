#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

//! WASI JSON document store (`wasi:jsondb`).

pub mod document_store;

#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(target_arch = "wasm32")]
pub mod store {
    //! Store operations for WASM guests (mirrors `omnia_wasi_sql` top-level modules).

    pub use crate::guest::store::*;
}

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub use host::*;
