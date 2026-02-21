#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg(target_arch = "wasm32")]

mod delete;
mod entity;
mod filter;
mod insert;
mod join;
mod query;
mod select;
mod update;

pub use delete::DeleteBuilder;
pub use entity::{Entity, EntityValues, FetchValue};
pub use filter::Filter;
pub use insert::InsertBuilder;
pub use join::Join;
// Re-export basic WASI SQL types for use in query parameters and custom value conversions.
pub use omnia_wasi_sql::{DataType, Field, Row};
pub use select::SelectBuilder;
pub use update::UpdateBuilder;

// Re-exports for ``entity`` macro use only. This is needed to avoid leaking ``SeaQuery`` value
// types into guest code
#[doc(hidden)]
pub mod __private {
    pub use sea_query::Value;
}
